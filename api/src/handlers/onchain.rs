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
