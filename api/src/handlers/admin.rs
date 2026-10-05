use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    models::{ModerationRequest, UserData},
    services::firestore::QueryFilter,
    AppState,
};

/// Guards an admin endpoint — returns 403 if `auth.uid` is not in `ADMIN_UIDS`.
fn ensure_admin(state: &AppState, auth: &AuthenticatedUser) -> Result<(), AppError> {
    if state.config.is_admin(&auth.uid) {
        Ok(())
    } else {
        Err(AppError::Forbidden("Admin access required".to_owned()))
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// "pending" | "approved" | "rejected" | "banned" | "all". Defaults to "pending".
    pub status: Option<String>,
}

/// `GET /api/admin/companies?status=pending`
pub async fn list_companies(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<UserData>>> {
    ensure_admin(&state, &auth)?;

    let status = q.status.as_deref().unwrap_or("pending");

    // Firestore queries with multiple inequality filters are limited, so we
    // pick a single discriminating filter per status bucket and rely on
    // serde defaults (`banned = false`, `pending_approval = false`, etc.)
    // for the remaining fields.
    let filters: Vec<QueryFilter> = match status {
        "pending"  => vec![QueryFilter::equal("pendingApproval", true)],
        "approved" => vec![QueryFilter::equal("companyApproved", true)],
        "rejected" => vec![QueryFilter::equal("approvalStatus",  "rejected")],
        "banned"   => vec![QueryFilter::equal("banned",          true)],
        "all"      => vec![],
        other => {
            return Err(AppError::BadRequest(format!(
                "Unknown status filter '{}'. Use pending|approved|rejected|banned|all.",
                other
            )))
        }
    };

    // No Firestore-side orderBy — a (where, orderBy) pair on different fields
    // requires a composite index. We sort in memory after fetching; with the
    // 200-row cap this is negligible and avoids any console-side setup.
    let docs = state
        .firestore
        .query("users", filters, None, Some(200))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    // Deserialize, skipping any malformed legacy docs rather than failing the whole call.
    let mut users: Vec<UserData> = docs
        .into_iter()
        .filter_map(|d| serde_json::from_value::<UserData>(d).ok())
        .collect();

    // Newest applications first; users with no request timestamp sink to the bottom.
    users.sort_by(|a, b| {
        b.approval_requested_at
            .as_deref()
            .unwrap_or("")
            .cmp(a.approval_requested_at.as_deref().unwrap_or(""))
    });

    Ok(Json(users))
}

/// `GET /api/admin/companies/:uid` — full record for a single user.
pub async fn get_company(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
) -> ApiResult<Json<UserData>> {
    ensure_admin(&state, &auth)?;

    let doc = state
        .firestore
        .get("users", &uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("User {} not found", uid)))?;

    let user: UserData =
        serde_json::from_value(doc).map_err(|e| AppError::Internal(e.into()))?;
    Ok(Json(user))
}

async fn apply_moderation(
    state: &AppState,
    actor_uid: &str,
    target_uid: &str,
    patch: serde_json::Value,
) -> Result<(), AppError> {
    let mut obj = patch.as_object().cloned().unwrap_or_default();
    obj.insert("reviewedAt".to_owned(), json!(chrono::Utc::now().to_rfc3339()));
    obj.insert("reviewedBy".to_owned(), json!(actor_uid));

    state
        .firestore
        .update("users", target_uid, &serde_json::Value::Object(obj))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(())
}

/// `POST /api/admin/companies/:uid/approve`
pub async fn approve(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
) -> ApiResult<StatusCode> {
    ensure_admin(&state, &auth)?;
    apply_moderation(
        &state,
        &auth.uid,
        &uid,
        json!({
            "companyApproved": true,
            "pendingApproval": false,
            "banned":          false,
            "approvalStatus":  "approved",
            "rejectionReason": serde_json::Value::Null,
            "banReason":       serde_json::Value::Null,
        }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/admin/companies/:uid/reject`
pub async fn reject(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
    Json(body): Json<ModerationRequest>,
) -> ApiResult<StatusCode> {
    ensure_admin(&state, &auth)?;
    apply_moderation(
        &state,
        &auth.uid,
        &uid,
        json!({
            "companyApproved": false,
            "pendingApproval": false,
            "approvalStatus":  "rejected",
            "rejectionReason": body.reason.unwrap_or_default(),
        }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/admin/companies/:uid/ban`
pub async fn ban(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
    Json(body): Json<ModerationRequest>,
) -> ApiResult<StatusCode> {
    ensure_admin(&state, &auth)?;
    if auth.uid == uid {
        return Err(AppError::BadRequest(
            "Cannot ban yourself".to_owned(),
        ));
    }
    apply_moderation(
        &state,
        &auth.uid,
        &uid,
        json!({
            "banned":          true,
            "companyApproved": false,
            "pendingApproval": false,
            "approvalStatus":  "banned",
            "banReason":       body.reason.unwrap_or_default(),
        }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/admin/companies/:uid/unban`
pub async fn unban(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
) -> ApiResult<StatusCode> {
    ensure_admin(&state, &auth)?;
    apply_moderation(
        &state,
        &auth.uid,
        &uid,
        json!({
            "banned":         false,
            "approvalStatus": serde_json::Value::Null,
            "banReason":      serde_json::Value::Null,
        }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/admin/me` — lightweight ping that confirms the caller is an admin.
pub async fn me(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<serde_json::Value>> {
    ensure_admin(&state, &auth)?;
    Ok(Json(json!({ "uid": auth.uid, "admin": true })))
}
