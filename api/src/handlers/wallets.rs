use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    models::{
        AddCryptoWalletRequest, CryptoWallet, MarkiWalletView, UpdateEmailRequest,
        UpdateFingerprintRequest, WalletType,
    },
    AppState,
};

// ── Marki wallet ──────────────────────────────────────────────────────────────

/// `GET /api/wallets/marki`
pub async fn get_marki_wallet(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<MarkiWalletView>> {
    let doc = state
        .firestore
        .get("wallets", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Marki wallet not found".to_owned()))?;

    let wallet: crate::models::MarkiWallet =
        serde_json::from_value(doc).map_err(|e| AppError::Internal(e.into()))?;

    Ok(Json(MarkiWalletView::from(wallet)))
}

/// `PUT /api/wallets/marki/fingerprint`
pub async fn update_fingerprint(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(body): Json<UpdateFingerprintRequest>,
) -> ApiResult<StatusCode> {
    state
        .firestore
        .update(
            "wallets",
            &auth.uid,
            &json!({ "fingerprintEnabled": body.enabled }),
        )
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/wallets/marki/email`
/// Updates the user's Firebase Auth email via the REST API using their own token.
pub async fn update_email(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(body): Json<UpdateEmailRequest>,
) -> ApiResult<StatusCode> {
    let url = format!(
        "https://identitytoolkit.googleapis.com/v1/accounts:update?key={}",
        state.config.firebase_api_key
    );

    let http = reqwest::Client::new();
    let resp = http
        .post(&url)
        .json(&json!({
            "idToken": auth.token,
            "email": body.new_email,
            "returnSecureToken": true,
        }))
        .send()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;

    if !resp.status().is_success() {
        let err = resp.text().await.unwrap_or_default();
        tracing::warn!(error = %err, "Firebase email update failed");
        return Err(AppError::BadRequest("Failed to update email".to_owned()));
    }

    // Mirror email change in Firestore user doc.
    state
        .firestore
        .update("users", &auth.uid, &json!({ "email": body.new_email }))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Crypto wallets ────────────────────────────────────────────────────────────

/// Read the `wallets` array from `crypto_wallets/{uid}`.
async fn load_crypto_wallets(
    state: &AppState,
    uid: &str,
) -> ApiResult<Vec<CryptoWallet>> {
    let doc = state
        .firestore
        .get("crypto_wallets", uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({ "wallets": [] }));

    let wallets: Vec<CryptoWallet> = doc
        .get("wallets")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    Ok(wallets)
}

/// Persist the `wallets` array back to `crypto_wallets/{uid}`.
async fn save_crypto_wallets(
    state: &AppState,
    uid: &str,
    wallets: &[CryptoWallet],
) -> ApiResult<()> {
    state
        .firestore
        .update("crypto_wallets", uid, &json!({ "wallets": wallets }))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(())
}

/// `GET /api/wallets/crypto`
pub async fn get_crypto_wallets(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<CryptoWallet>>> {
    let wallets = load_crypto_wallets(&state, &auth.uid).await?;
    Ok(Json(wallets))
}

/// `POST /api/wallets/crypto`
/// Validates the Solana address in Rust, then appends the wallet.
pub async fn add_crypto_wallet(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(body): Json<AddCryptoWalletRequest>,
) -> ApiResult<(StatusCode, Json<CryptoWallet>)> {
    if !state.solana.is_valid_address(&body.address) {
        return Err(AppError::BadRequest(format!(
            "'{}' is not a valid Solana base58 address (expected 32-byte decoded length)",
            body.address
        )));
    }

    let mut wallets = load_crypto_wallets(&state, &auth.uid).await?;

    // Prevent duplicate addresses.
    if wallets.iter().any(|w| w.address == body.address) {
        return Err(AppError::Conflict(
            "This wallet address is already connected".to_owned(),
        ));
    }

    // Fetch live balance from Solana RPC.
    let balance = state
        .solana
        .get_balance(&body.address)
        .await
        .unwrap_or(0.0);

    let now = Utc::now().to_rfc3339();
    let wallet = CryptoWallet {
        id: Uuid::new_v4().to_string(),
        wallet_type: WalletType::Phantom,
        address: body.address,
        network: "solana".to_owned(),
        balance,
        currency: "SOL".to_owned(),
        is_connected: true,
        connected_at: now.clone(),
        last_used: now,
        label: body.label,
    };

    wallets.push(wallet.clone());
    save_crypto_wallets(&state, &auth.uid, &wallets).await?;

    Ok((StatusCode::CREATED, Json(wallet)))
}

/// `DELETE /api/wallets/crypto/:id`
pub async fn remove_crypto_wallet(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(wallet_id): Path<String>,
) -> ApiResult<StatusCode> {
    let mut wallets = load_crypto_wallets(&state, &auth.uid).await?;
    let len_before = wallets.len();
    wallets.retain(|w| w.id != wallet_id);

    if wallets.len() == len_before {
        return Err(AppError::NotFound(format!("Wallet {} not found", wallet_id)));
    }

    save_crypto_wallets(&state, &auth.uid, &wallets).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/wallets/crypto/:id/balance`
/// Fetches fresh balance from Solana RPC and persists it.
pub async fn refresh_wallet_balance(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(wallet_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let mut wallets = load_crypto_wallets(&state, &auth.uid).await?;

    let wallet = wallets
        .iter_mut()
        .find(|w| w.id == wallet_id)
        .ok_or_else(|| AppError::NotFound(format!("Wallet {} not found", wallet_id)))?;

    let balance = state
        .solana
        .get_balance(&wallet.address)
        .await
        .map_err(|e| AppError::Solana(e.to_string()))?;

    wallet.balance = balance;
    wallet.last_used = Utc::now().to_rfc3339();

    save_crypto_wallets(&state, &auth.uid, &wallets).await?;
    Ok(Json(json!({ "balance": balance, "currency": "SOL" })))
}
