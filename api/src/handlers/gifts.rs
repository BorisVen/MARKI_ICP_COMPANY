//! Gifts: an owner (company or user) turns NFTs from their Marki wallet into a
//! gift link. Whoever opens the link and signs in claims one NFT; it moves into
//! their wallet. Every claim is recorded on the gift, so the sender sees what
//! went to whom and when. Ownership moves inside Marki only (no on-chain transfer).
//!
//! Firestore collection `gifts/{code}`; `code` is the public, unguessable id in
//! the link. Claims are serialized by a process-wide lock: the API runs as one
//! instance, and moving an NFT is a read-modify-write of two wallet arrays.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    errors::{ApiResult, AppError},
    handlers::nfts::{load_nfts, save_nfts},
    middleware::auth::AuthenticatedUser,
    models::{NotificationMeta, NotificationType},
    services::firestore::QueryFilter,
    AppState,
};

const MAX_ITEMS: usize = 1_000;
const MAX_MESSAGE: usize = 280;

static CLAIM_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GiftClaim {
    /// NFT id in the receiver's wallet.
    pub nft_id: String,
    /// NFT id it had in the sender's wallet.
    pub source_nft_id: String,
    pub uid: String,
    pub name: String,
    pub username: String,
    pub claimed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gift {
    pub id: String,
    pub owner_id: String,
    pub owner_name: String,
    pub owner_username: String,
    pub from_company: bool,
    pub title: String,
    pub message: String,
    pub image: String,
    pub total: usize,
    /// Sender-wallet NFT ids not claimed yet.
    pub remaining_nft_ids: Vec<String>,
    pub claims: Vec<GiftClaim>,
    /// Receiver uids, for "gifts I received" queries.
    pub claimed_by: Vec<String>,
    /// "active" | "completed" | "cancelled"
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGiftRequest {
    pub nft_ids: Vec<String>,
    pub title: Option<String>,
    pub message: Option<String>,
}

/// What anyone with the link sees before signing in.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GiftPreview {
    pub id: String,
    pub title: String,
    pub message: String,
    pub image: String,
    pub owner_name: String,
    pub owner_username: String,
    pub from_company: bool,
    pub total: usize,
    pub remaining: usize,
    pub status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyGifts {
    pub sent: Vec<Gift>,
    pub received: Vec<Gift>,
}

fn new_code() -> String {
    // 16 hex chars from a v4 UUID: unguessable enough for a claim link.
    Uuid::new_v4().simple().to_string()[..16].to_owned()
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

async fn load_gift(state: &AppState, code: &str) -> ApiResult<Gift> {
    let doc = state
        .firestore
        .get("gifts", code)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Gift not found".into()))?;
    serde_json::from_value(doc).map_err(|e| AppError::Internal(e.into()))
}

async fn save_gift(state: &AppState, gift: &Gift) -> ApiResult<()> {
    let value = serde_json::to_value(gift).map_err(|e| AppError::Internal(e.into()))?;
    state
        .firestore
        .set("gifts", &gift.id, &value)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))
}

async fn user_doc(state: &AppState, uid: &str) -> ApiResult<Value> {
    Ok(state
        .firestore
        .get("users", uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({})))
}

async fn notify(state: &AppState, user_id: &str, title: String, text: String, nft_title: &str) {
    let id = Uuid::new_v4().to_string();
    let doc = json!({
        "id": id,
        "userId": user_id,
        "type": NotificationType::Gift,
        "title": title,
        "text": text,
        "read": false,
        "createdAt": Utc::now().to_rfc3339(),
        "metadata": NotificationMeta { nft_title: Some(nft_title.to_owned()), ..Default::default() },
    });
    if let Err(e) = state.firestore.set("notifications", &id, &doc).await {
        tracing::warn!(error = %e, "Failed to create gift notification");
    }
}

/// NFT ids already promised in another active gift of this owner.
async fn reserved_ids(state: &AppState, owner: &str) -> ApiResult<Vec<String>> {
    let docs = state
        .firestore
        .query("gifts", vec![QueryFilter::equal("ownerId", owner.to_owned())], None, None)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(docs
        .into_iter()
        .filter_map(|d| serde_json::from_value::<Gift>(d).ok())
        .filter(|g| g.status == "active")
        .flat_map(|g| g.remaining_nft_ids)
        .collect())
}

/// `POST /api/gifts` — turn NFTs from the caller's wallet into a gift link.
pub async fn create_gift(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(body): Json<CreateGiftRequest>,
) -> ApiResult<(StatusCode, Json<Gift>)> {
    let mut ids = body.nft_ids;
    ids.sort();
    ids.dedup();
    if ids.is_empty() || ids.len() > MAX_ITEMS {
        return Err(AppError::BadRequest(format!("Choose 1–{MAX_ITEMS} NFTs")));
    }
    let message = body.message.unwrap_or_default().trim().to_owned();
    if message.chars().count() > MAX_MESSAGE {
        return Err(AppError::BadRequest(format!("Message is longer than {MAX_MESSAGE} characters")));
    }

    let wallet = load_nfts(&state, &auth.uid).await?;
    let reserved = reserved_ids(&state, &auth.uid).await?;
    let mut picked = Vec::with_capacity(ids.len());
    for id in &ids {
        let nft = wallet
            .iter()
            .find(|n| &n.id == id)
            .ok_or_else(|| AppError::BadRequest(format!("NFT {id} is not in your wallet")))?;
        if nft.for_sale {
            return Err(AppError::BadRequest(format!("\"{}\" is on sale; remove it from sale first", nft.title)));
        }
        // The edition master is the original the copies are printed from.
        if nft.edition_number == Some(0) && nft.edition_count.is_some() {
            return Err(AppError::BadRequest(format!("\"{}\" is the edition original; gift its copies instead", nft.title)));
        }
        if reserved.contains(id) {
            return Err(AppError::Conflict(format!("\"{}\" is already in another gift", nft.title)));
        }
        picked.push(nft.clone());
    }

    let owner = user_doc(&state, &auth.uid).await?;
    let first = &picked[0];
    let title = body
        .title
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .or_else(|| first.batch_name.clone())
        .unwrap_or_else(|| first.title.clone());
    let now = Utc::now().to_rfc3339();
    let gift = Gift {
        id: new_code(),
        owner_id: auth.uid.clone(),
        owner_name: Some(text(&owner, "companyName"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| text(&owner, "name")),
        owner_username: text(&owner, "username"),
        from_company: owner.get("companyApproved").and_then(Value::as_bool).unwrap_or(false),
        title,
        message,
        image: first.image.clone(),
        total: picked.len(),
        remaining_nft_ids: picked.iter().map(|n| n.id.clone()).collect(),
        claims: vec![],
        claimed_by: vec![],
        status: "active".into(),
        created_at: now.clone(),
        updated_at: now,
    };
    save_gift(&state, &gift).await?;
    Ok((StatusCode::CREATED, Json(gift)))
}

/// `GET /api/gifts` — gifts I sent (with every claim) and gifts I received.
pub async fn my_gifts(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<MyGifts>> {
    let parse = |docs: Vec<Value>| -> Vec<Gift> {
        let mut gifts: Vec<Gift> = docs.into_iter().filter_map(|d| serde_json::from_value(d).ok()).collect();
        gifts.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        gifts
    };
    let sent = state
        .firestore
        .query("gifts", vec![QueryFilter::equal("ownerId", auth.uid.clone())], None, None)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let received = state
        .firestore
        .query("gifts", vec![QueryFilter::array_contains("claimedBy", auth.uid.clone())], None, None)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(Json(MyGifts { sent: parse(sent), received: parse(received) }))
}

/// `GET /api/public/gifts/:code` — preview for the claim page (no sign-in).
pub async fn preview_gift(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> ApiResult<Json<GiftPreview>> {
    let g = load_gift(&state, &code).await?;
    Ok(Json(GiftPreview {
        remaining: g.remaining_nft_ids.len(),
        id: g.id,
        title: g.title,
        message: g.message,
        image: g.image,
        owner_name: g.owner_name,
        owner_username: g.owner_username,
        from_company: g.from_company,
        total: g.total,
        status: g.status,
    }))
}

/// `POST /api/gifts/:code/claim` — take one NFT from the gift into my wallet.
pub async fn claim_gift(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(code): Path<String>,
) -> ApiResult<Json<Value>> {
    let _guard = CLAIM_LOCK.lock().await;
    let mut gift = load_gift(&state, &code).await?;
    if gift.status == "cancelled" {
        return Err(AppError::BadRequest("This gift was cancelled".into()));
    }
    if gift.owner_id == auth.uid {
        return Err(AppError::BadRequest("You cannot claim your own gift".into()));
    }
    if gift.claimed_by.contains(&auth.uid) {
        return Err(AppError::Conflict("You already claimed this gift".into()));
    }

    let mut owner_wallet = load_nfts(&state, &gift.owner_id).await?;
    // Skip ids the owner no longer has (sold or deleted since the gift was made).
    let pick = gift
        .remaining_nft_ids
        .iter()
        .position(|id| owner_wallet.iter().any(|n| &n.id == id));
    let Some(pick) = pick else {
        gift.remaining_nft_ids.clear();
        gift.status = "completed".into();
        gift.updated_at = Utc::now().to_rfc3339();
        save_gift(&state, &gift).await?;
        return Err(AppError::Conflict("All items of this gift are already taken".into()));
    };
    let source_id = gift.remaining_nft_ids.remove(pick);
    let pos = owner_wallet.iter().position(|n| n.id == source_id).unwrap();
    let source = owner_wallet.remove(pos);

    let me = user_doc(&state, &auth.uid).await?;
    let my_name = text(&me, "name");
    let new_id = Uuid::new_v4().to_string();
    let username = text(&me, "username");
    let owners = source.owners_with(&auth.uid, &my_name, (!username.is_empty()).then_some(username), "gift", &Utc::now().to_rfc3339());
    let received = crate::models::Nft {
        owners,
        id: new_id.clone(),
        owner_id: auth.uid.clone(),
        owner_name: my_name.clone(),
        for_sale: false,
        price: None,
        gift_from: Some(gift.owner_name.clone()),
        gift_id: Some(gift.id.clone()),
        ..source
    };
    let nft_title = received.title.clone();

    save_nfts(&state, &gift.owner_id, &owner_wallet).await?;
    let mut my_wallet = load_nfts(&state, &auth.uid).await?;
    my_wallet.push(received.clone());
    save_nfts(&state, &auth.uid, &my_wallet).await?;

    let now = Utc::now().to_rfc3339();
    gift.claims.push(GiftClaim {
        nft_id: new_id,
        source_nft_id: source_id,
        uid: auth.uid.clone(),
        name: my_name.clone(),
        username: text(&me, "username"),
        claimed_at: now.clone(),
    });
    gift.claimed_by.push(auth.uid.clone());
    if gift.remaining_nft_ids.is_empty() {
        gift.status = "completed".into();
    }
    gift.updated_at = now;
    save_gift(&state, &gift).await?;

    notify(&state, &auth.uid, "You received a gift 🎁".into(),
        format!("\"{}\" from {} is now in your passports", nft_title, gift.owner_name), &nft_title).await;
    notify(&state, &gift.owner_id, "Your gift was claimed 🎁".into(),
        format!("{} claimed \"{}\" ({} of {} left)", my_name, nft_title, gift.remaining_nft_ids.len(), gift.total), &nft_title).await;

    Ok(Json(json!({ "gift": gift, "nft": received })))
}

/// `DELETE /api/gifts/:code` — stop the link; unclaimed NFTs stay with the owner.
pub async fn cancel_gift(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(code): Path<String>,
) -> ApiResult<Json<Gift>> {
    let _guard = CLAIM_LOCK.lock().await;
    let mut gift = load_gift(&state, &code).await?;
    if gift.owner_id != auth.uid {
        return Err(AppError::Forbidden("Only the sender can cancel this gift".into()));
    }
    if gift.status == "active" {
        gift.status = "cancelled".into();
        gift.remaining_nft_ids.clear();
        gift.updated_at = Utc::now().to_rfc3339();
        save_gift(&state, &gift).await?;
    }
    Ok(Json(gift))
}

#[cfg(test)]
mod tests {
    use super::new_code;

    #[test]
    fn codes_are_16_hex_and_unique() {
        let a = new_code();
        let b = new_code();
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
