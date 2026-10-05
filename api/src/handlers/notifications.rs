use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use serde_json::json;
use std::sync::Arc;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    models::Notification,
    services::firestore::QueryFilter,
    AppState,
};

/// `GET /api/notifications`
/// Returns all notifications for the authenticated user, newest first.
/// Sorting is done in-memory to avoid requiring a Firestore composite index.
pub async fn get_notifications(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<Notification>>> {
    let docs = state
        .firestore
        .query(
            "notifications",
            vec![QueryFilter::equal("userId", auth.uid.as_str())],
            None,  // no server-side orderBy — avoids FAILED_PRECONDITION index error
            None,
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    let mut notifications: Vec<Notification> = docs
        .into_iter()
        .filter_map(|d| serde_json::from_value(d).ok())
        .collect();

    // Sort newest-first in memory.
    notifications.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    notifications.truncate(100);

    Ok(Json(notifications))
}

/// `PUT /api/notifications/:id/read`
pub async fn mark_read(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(notification_id): Path<String>,
) -> ApiResult<StatusCode> {
    // Verify ownership before update.
    let doc = state
        .firestore
        .get("notifications", &notification_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Notification not found".to_owned()))?;

    let owner = doc
        .get("userId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if owner != auth.uid {
        return Err(AppError::Forbidden(
            "Cannot modify another user's notification".to_owned(),
        ));
    }

    state
        .firestore
        .update("notifications", &notification_id, &json!({ "read": true }))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/notifications/read-all`
/// Marks every unread notification for the current user as read.
pub async fn mark_all_read(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<serde_json::Value>> {
    // Query only by userId — filtering by read=false in-memory avoids
    // requiring a composite Firestore index (FAILED_PRECONDITION).
    let docs = state
        .firestore
        .query(
            "notifications",
            vec![QueryFilter::equal("userId", auth.uid.as_str())],
            None,
            None,
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    let mut updated = 0usize;
    for doc in &docs {
        let already_read = doc.get("read").and_then(|v| v.as_bool()).unwrap_or(false);
        if already_read {
            continue;
        }
        if let Some(id) = doc.get("id").and_then(|v| v.as_str()) {
            let _ = state
                .firestore
                .update("notifications", id, &json!({ "read": true }))
                .await;
            updated += 1;
        }
    }

    Ok(Json(json!({ "updated": updated })))
}

/// `DELETE /api/notifications/:id`
pub async fn delete_notification(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(notification_id): Path<String>,
) -> ApiResult<StatusCode> {
    let doc = state
        .firestore
        .get("notifications", &notification_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Notification not found".to_owned()))?;

    let owner = doc
        .get("userId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if owner != auth.uid {
        return Err(AppError::Forbidden(
            "Cannot delete another user's notification".to_owned(),
        ));
    }

    state
        .firestore
        .delete("notifications", &notification_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
