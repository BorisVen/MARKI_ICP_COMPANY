//! On-chain handover of gifted NFTs.
//!
//! Gifts move NFTs inside Marki at once, but the ICP token stays with the
//! sender's principal until the sender signs an `icrc7_transfer` in Plug. Users
//! link their Plug principal to the profile; the sender's app then lists the
//! tokens it still holds for people who have one, and transfers them in one go.
//! The canister is the source of truth for who holds a token: the app checks
//! `icrc7_owner_of` and only offers tokens still owned by the connected wallet.

use axum::{extract::State, http::StatusCode, Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    errors::{ApiResult, AppError},
    handlers::nfts::load_nfts,
    middleware::auth::AuthenticatedUser,
    services::firestore::QueryFilter,
    AppState,
};

fn fs(e: impl std::fmt::Display) -> AppError {
    AppError::Firebase(e.to_string())
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

/// Rough shape check of a textual principal: dash-separated base32 groups.
pub fn looks_like_principal(p: &str) -> bool {
    (10..=63).contains(&p.len())
        && p.split('-').all(|g| !g.is_empty() && g.len() <= 5 && g.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkRequest {
    /// `None` unlinks.
    pub principal: Option<String>,
}

/// `PUT /api/me/icp-principal` — link (or unlink) the Plug principal.
pub async fn link_principal(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(req): Json<LinkRequest>,
) -> ApiResult<StatusCode> {
    let principal = req.principal.map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
    if let Some(p) = &principal {
        if !looks_like_principal(p) || p == "2vxsx-fae" {
            return Err(AppError::BadRequest("Це не схоже на principal гаманця".into()));
        }
    }
    state.firestore.update("users", &auth.uid, &json!({ "icpPrincipal": principal })).await.map_err(fs)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingTransfer {
    /// `icp:<canister>:<token>` of the gifted NFT.
    pub mint_address: String,
    pub nft_title: String,
    pub to_uid: String,
    pub to_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_principal: Option<String>,
    pub given_at: String,
}

/// `GET /api/onchain/pending` — NFTs I gave away (links and requests) whose
/// receivers may still need the token on-chain. The app filters by the
/// canister's current owners.
pub async fn pending(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<PendingTransfer>>> {
    // (receiver uid, receiver's nft id, when)
    let mut given: Vec<(String, String, String)> = Vec::new();
    let gifts = state.firestore.query("gifts", vec![QueryFilter::equal("ownerId", auth.uid.clone())], None, None).await.map_err(fs)?;
    for g in &gifts {
        for c in g.get("claims").and_then(Value::as_array).into_iter().flatten() {
            given.push((text(c, "uid"), text(c, "nftId"), text(c, "claimedAt")));
        }
    }
    let requests = state.firestore.query("gift_requests", vec![QueryFilter::equal("ownerId", auth.uid.clone())], None, None).await.map_err(fs)?;
    for r in requests.iter().filter(|r| text(r, "status") == "approved") {
        given.push((text(r, "requesterId"), text(r, "givenNftId"), text(r, "decidedAt")));
    }

    let mut wallets: HashMap<String, Vec<crate::models::Nft>> = HashMap::new();
    let mut people: HashMap<String, Value> = HashMap::new();
    let mut out = Vec::new();
    for (uid, nft_id, at) in given {
        if uid.is_empty() || nft_id.is_empty() {
            continue;
        }
        if !wallets.contains_key(&uid) {
            wallets.insert(uid.clone(), load_nfts(&state, &uid).await.unwrap_or_default());
        }
        // Gone from the receiver's wallet (passed on again): not ours to send.
        let Some(nft) = wallets[&uid].iter().find(|n| n.id == nft_id) else { continue };
        let Some(mint) = nft.mint_address.clone().filter(|m| m.starts_with("icp:")) else { continue };
        if !people.contains_key(&uid) {
            let doc = state.firestore.get("users", &uid).await.ok().flatten().unwrap_or_else(|| json!({}));
            people.insert(uid.clone(), doc);
        }
        let person = &people[&uid];
        let principal = text(person, "icpPrincipal");
        out.push(PendingTransfer {
            mint_address: mint,
            nft_title: nft.title.clone(),
            to_name: text(person, "name"),
            to_principal: (!principal.is_empty()).then_some(principal),
            to_uid: uid,
            given_at: at,
        });
    }
    out.sort_by(|a, b| b.given_at.cmp(&a.given_at));
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::looks_like_principal;

    #[test]
    fn principal_shape() {
        assert!(looks_like_principal("pztgf-ur626-w6phw-7fqfr-6rdz7-fjdbo-pqot5-26nzn-qparg-5l7vn-tae"));
        assert!(!looks_like_principal("Not A Principal"));
        assert!(!looks_like_principal("abc"));
        assert!(!looks_like_principal("pztgf--ur626"));
    }
}

// ── Marki custody ───────────────────────────────────────────────────────────

/// Moves the tokens of NFTs just gifted from the sender's custody sub-account to
/// the receiver's, in the background. Tokens held elsewhere (e.g. a brand's own
/// Plug) are left for the sender to hand over from their wallet.
pub fn spawn_custody_move(from_uid: String, to_uid: String, mint_addresses: Vec<String>) {
    tokio::spawn(async move {
        let Some(c) = crate::services::icp::custody().await else { return };
        let ids: Vec<u64> = mint_addresses.iter().filter_map(|m| crate::services::icp::token_of(m, &c.canister())).collect();
        if ids.is_empty() {
            return;
        }
        match c.transfer(&from_uid, &c.account_of(&to_uid), &ids).await {
            Ok(res) => {
                let moved = res.iter().filter(|r| r.is_ok()).count();
                tracing::info!(moved, total = ids.len(), "custody move for a gift");
            }
            Err(e) => tracing::warn!(error = %e, "custody move failed"),
        }
    });
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustodyAccount {
    pub custodian: String,
    /// Hex of the caller's 32-byte custody sub-account.
    pub subaccount: String,
}

/// `GET /api/onchain/account` — where the caller's tokens live in Marki custody.
pub async fn account(Extension(auth): Extension<AuthenticatedUser>) -> ApiResult<Json<CustodyAccount>> {
    let c = crate::services::icp::custody().await.ok_or_else(|| AppError::NotFound("Marki custody is off".into()))?;
    Ok(Json(CustodyAccount {
        custodian: c.custodian.to_text(),
        subaccount: hex::encode(crate::services::icp::user_subaccount(&auth.uid)),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawRequest {
    /// The user's own wallet (Internet Identity or Plug).
    pub principal: String,
    /// NFTs to withdraw; all of the caller's on-chain NFTs when empty.
    #[serde(default)]
    pub nft_ids: Vec<String>,
}

/// `POST /api/onchain/withdraw` — move my tokens from Marki custody to my wallet.
pub async fn withdraw(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(req): Json<WithdrawRequest>,
) -> ApiResult<Json<Value>> {
    let c = crate::services::icp::custody().await.ok_or_else(|| AppError::BadRequest("Сховище Marki вимкнене".into()))?;
    let to = candid::Principal::from_text(req.principal.trim())
        .map_err(|_| AppError::BadRequest("Це не схоже на principal гаманця".into()))?;
    if to == candid::Principal::anonymous() || to == c.custodian {
        return Err(AppError::BadRequest("Неправильний гаманець".into()));
    }
    let nfts = load_nfts(&state, &auth.uid).await?;
    let ids: Vec<u64> = nfts
        .iter()
        .filter(|n| req.nft_ids.is_empty() || req.nft_ids.contains(&n.id))
        .filter_map(|n| n.mint_address.as_deref().and_then(|m| crate::services::icp::token_of(m, &c.canister())))
        .collect();
    if ids.is_empty() {
        return Ok(Json(json!({ "moved": 0, "total": 0 })));
    }
    let res = c
        .transfer(&auth.uid, &crate::services::icp::Account { owner: to, subaccount: None }, &ids)
        .await
        .map_err(|e| AppError::Internal(e))?;
    let moved = res.iter().filter(|r| r.is_ok()).count();
    Ok(Json(json!({ "moved": moved, "total": ids.len() })))
}

const WALLET_KEYS: &str = "marki_wallet_keys";

/// `GET /api/me/icp-wallet` — the seed of the user's built-in Marki wallet.
///
/// Phones have no Plug, so the app signs canister calls (plans, mints) with an
/// Ed25519 key made from this seed. One seed per Marki account, the same on
/// every device; its tokens still go to Marki custody.
pub async fn wallet_seed(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Value>> {
    if let Some(doc) = state.firestore.get(WALLET_KEYS, &auth.uid).await.map_err(fs)? {
        if let Some(seed) = doc["seed"].as_str() {
            return Ok(Json(json!({ "seed": seed })));
        }
    }
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    let seed = hex::encode(bytes);
    state.firestore.set(WALLET_KEYS, &auth.uid, &json!({ "seed": seed, "createdAt": chrono::Utc::now().to_rfc3339() }))
        .await.map_err(fs)?;
    Ok(Json(json!({ "seed": seed })))
}

#[derive(Debug, Deserialize)]
pub struct PushTokenRequest {
    pub token: String,
}

/// `PUT /api/me/push-token` — this phone wants push notifications.
pub async fn register_push_token(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(req): Json<PushTokenRequest>,
) -> ApiResult<StatusCode> {
    let token = req.token.trim().to_owned();
    if token.len() < 20 || token.len() > 4096 {
        return Err(AppError::BadRequest("bad push token".into()));
    }
    let mut tokens = crate::services::push::tokens_of(&state.firestore, &auth.uid).await;
    tokens.retain(|t| *t != token);
    tokens.push(token);
    let keep = tokens.len().saturating_sub(10);
    state.firestore.set(crate::services::push::TOKENS, &auth.uid, &json!({ "tokens": &tokens[keep..] })).await.map_err(fs)?;
    Ok(StatusCode::NO_CONTENT)
}
