use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use serde_json::json;
use std::sync::Arc;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    models::{
        ChangePasswordRequest, RequestApprovalRequest, SetApprovalRequest, UpdateProfileRequest,
        UserData,
    },
    services::StorageClient,
    AppState,
};

fn ext_from_content_type(ct: &str) -> &str {
    match ct {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "jpg",
    }
}

/// `GET /api/profile/:uid`
pub async fn get_profile(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
) -> ApiResult<Json<UserData>> {
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

/// `PUT /api/profile/:uid`
pub async fn update_profile(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
    Json(body): Json<UpdateProfileRequest>,
) -> ApiResult<Json<UserData>> {
    if auth.uid != uid {
        return Err(AppError::Forbidden("Cannot update another user's profile".to_owned()));
    }

    let mut patch = serde_json::Map::new();
    if let Some(v) = body.name            { patch.insert("name".to_owned(),            json!(v)); }
    if let Some(v) = body.username        { patch.insert("username".to_owned(),        json!(v)); }
    if let Some(v) = body.bio             { patch.insert("bio".to_owned(),             json!(v)); }
    if let Some(v) = body.location        { patch.insert("location".to_owned(),        json!(v)); }
    if let Some(v) = body.delivery_address {
        patch.insert("deliveryAddress".to_owned(), json!(v));
    }

    if !patch.is_empty() {
        state
            .firestore
            .update("users", &uid, &serde_json::Value::Object(patch))
            .await
            .map_err(|e| AppError::Firebase(e.to_string()))?;
    }

    let doc = state
        .firestore
        .get("users", &uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("User not found after update".to_owned()))?;

    let user: UserData =
        serde_json::from_value(doc).map_err(|e| AppError::Internal(e.into()))?;
    Ok(Json(user))
}

/// `POST /api/profile/:uid/avatar`
/// Expects raw image bytes as request body; Content-Type header dictates format.
pub async fn upload_avatar(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    if auth.uid != uid {
        return Err(AppError::Forbidden("Cannot update another user's avatar".to_owned()));
    }

    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_owned();

    let ext = ext_from_content_type(&ct);
    let path = StorageClient::avatar_path(&uid, ext);

    let url = state
        .storage
        .upload(&path, body.to_vec(), &ct)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    state
        .firestore
        .update("users", &uid, &json!({ "avatar": url }))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok(Json(json!({ "avatar": url })))
}

/// `PUT /api/profile/:uid/password`
pub async fn change_password(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
    Json(body): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
    if auth.uid != uid {
        return Err(AppError::Forbidden("Cannot change another user's password".to_owned()));
    }
    if body.new_password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".to_owned(),
        ));
    }

    let url = format!(
        "https://identitytoolkit.googleapis.com/v1/accounts:update?key={}",
        state.config.firebase_api_key
    );

    let resp = reqwest::Client::new()
        .post(&url)
        .json(&json!({
            "idToken": auth.token,
            "password": body.new_password,
            "returnSecureToken": true,
        }))
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    if !resp.status().is_success() {
        let err = resp.text().await.unwrap_or_default();
        tracing::warn!(error = %err, "Firebase password change failed");
        return Err(AppError::BadRequest("Failed to change password".to_owned()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/profile/:uid/approval`
pub async fn set_approval(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
    Json(body): Json<SetApprovalRequest>,
) -> ApiResult<StatusCode> {
    state
        .firestore
        .update(
            "users",
            &uid,
            &json!({ "companyApproved": body.approved, "pendingApproval": false }),
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/profile/:uid/request-approval`
/// Saves the company application and marks the user as `pendingApproval=true`.
/// An admin reviews via `/api/admin/companies/:uid/{approve,reject,ban}`.
pub async fn request_approval(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(uid): Path<String>,
    Json(body): Json<RequestApprovalRequest>,
) -> ApiResult<StatusCode> {
    if auth.uid != uid {
        return Err(AppError::Forbidden(
            "Cannot request approval on behalf of another user".to_owned(),
        ));
    }

    let company_name = body.company_name.trim();
    let registration_number = body.registration_number.trim();
    let contact_email = body.contact_email.trim();
    if company_name.is_empty() || registration_number.is_empty() || contact_email.is_empty() {
        return Err(AppError::BadRequest(
            "companyName, registrationNumber, contactEmail are required".to_owned(),
        ));
    }

    // Re-applying after a rejection is allowed; banned users cannot re-apply.
    let existing = state
        .firestore
        .get("users", &uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    if let Some(doc) = existing {
        if doc.get("banned").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Err(AppError::Forbidden(
                "This account is banned and cannot submit applications".to_owned(),
            ));
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    state
        .firestore
        .update(
            "users",
            &uid,
            &json!({
                "companyApproved":     false,
                "pendingApproval":     true,
                "approvalStatus":      "pending",
                "companyName":         company_name,
                "registrationNumber":  registration_number,
                "contactEmail":        contact_email,
                "businessDescription": body.description.trim(),
                "approvalRequestedAt": now,
                "rejectionReason":     serde_json::Value::Null,
            }),
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
