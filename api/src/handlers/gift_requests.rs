//! Requests for NFTs from a gift collection, granted by hand.
//!
//! A gift-collection NFT's passport QR works like any passport (anyone sees the
//! details); a signed-in visitor may also ask for it, saying when and where they
//! bought the product. The owner reviews the request and gives the NFT away or
//! declines. Giving moves the NFT like a gift claim does (ownership chain,
//! notifications) and declines other pending requests for the same NFT.
//!
//! Firestore collection `gift_requests/{id}`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    errors::{ApiResult, AppError},
    handlers::{
        gifts::{notify, reserved_ids, text, user_doc, CLAIM_LOCK},
        nfts::{load_nfts, save_nfts},
    },
    middleware::auth::AuthenticatedUser,
    services::firestore::QueryFilter,
    AppState,
};

const REQUESTS: &str = "gift_requests";
const MAX_TEXT: usize = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GiftRequest {
    pub id: String,
    pub owner_id: String,
    pub nft_id: String,
    pub nft_title: String,
    #[serde(default)]
    pub nft_image: String,
    pub requester_id: String,
    pub requester_name: String,
    #[serde(default)]
    pub requester_username: String,
    /// When the requester bought the product (YYYY-MM-DD).
    pub bought_on: String,
    /// Shop or city.
    #[serde(default)]
    pub bought_at: String,
    #[serde(default)]
    pub comment: String,
    /// "pending" | "approved" | "declined"
    pub status: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
    /// The NFT's id in the requester's wallet after it was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub given_nft_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    pub owner_id: String,
    pub nft_id: String,
    pub bought_on: String,
    #[serde(default)]
    pub bought_at: String,
    #[serde(default)]
    pub comment: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyRequests {
    /// Requests for my NFTs.
    pub incoming: Vec<GiftRequest>,
    /// Requests I made.
    pub outgoing: Vec<GiftRequest>,
}

fn fs(e: impl std::fmt::Display) -> AppError {
    AppError::Firebase(e.to_string())
}

fn clean(s: &str) -> String {
    s.trim().chars().take(MAX_TEXT).collect()
}

async fn load(state: &AppState, id: &str) -> ApiResult<GiftRequest> {
    let doc = state.firestore.get(REQUESTS, id).await.map_err(fs)?
        .ok_or_else(|| AppError::NotFound("Запит не знайдено".into()))?;
    serde_json::from_value(doc).map_err(|e| AppError::Internal(e.into()))
}

async fn save(state: &AppState, r: &GiftRequest) -> ApiResult<()> {
    let v = serde_json::to_value(r).map_err(|e| AppError::Internal(e.into()))?;
    state.firestore.set(REQUESTS, &r.id, &v).await.map_err(fs)
}

async fn query(state: &AppState, field: &str, uid: &str) -> ApiResult<Vec<GiftRequest>> {
    let docs = state.firestore.query(REQUESTS, vec![QueryFilter::equal(field, uid.to_owned())], None, None).await.map_err(fs)?;
    let mut list: Vec<GiftRequest> = docs.into_iter().filter_map(|d| serde_json::from_value(d).ok()).collect();
    list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(list)
}

/// `POST /api/gift-requests` — ask the owner for a gift-collection NFT.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(req): Json<CreateRequest>,
) -> ApiResult<(StatusCode, Json<GiftRequest>)> {
    if req.owner_id == auth.uid {
        return Err(AppError::BadRequest("Це ваш NFT".into()));
    }
    let bought = NaiveDate::parse_from_str(req.bought_on.trim(), "%Y-%m-%d")
        .map_err(|_| AppError::BadRequest("Вкажіть дату покупки".into()))?;
    if bought > Utc::now().date_naive() {
        return Err(AppError::BadRequest("Дата покупки не може бути в майбутньому".into()));
    }
    let nft = load_nfts(&state, &req.owner_id).await?
        .into_iter()
        .find(|n| n.id == req.nft_id)
        .ok_or_else(|| AppError::NotFound("Цей NFT уже передано або видалено".into()))?;
    if !nft.gift_collection {
        return Err(AppError::BadRequest("Цей NFT не з подарункової колекції".into()));
    }
    let mine = query(&state, "requesterId", &auth.uid).await?;
    if mine.iter().any(|r| r.nft_id == req.nft_id && r.status == "pending") {
        return Err(AppError::Conflict("Ви вже надіслали запит на цей NFT — чекайте відповіді".into()));
    }

    let me = user_doc(&state, &auth.uid).await?;
    let request = GiftRequest {
        id: Uuid::new_v4().simple().to_string(),
        owner_id: req.owner_id.clone(),
        nft_id: nft.id.clone(),
        nft_title: nft.title.clone(),
        nft_image: nft.image.clone(),
        requester_id: auth.uid.clone(),
        requester_name: text(&me, "name"),
        requester_username: text(&me, "username"),
        bought_on: bought.format("%Y-%m-%d").to_string(),
        bought_at: clean(&req.bought_at),
        comment: clean(&req.comment),
        status: "pending".into(),
        created_at: Utc::now().to_rfc3339(),
        decided_at: None,
        given_nft_id: None,
    };
    save(&state, &request).await?;
    notify(&state, &req.owner_id, "Запит на подарунок 🎁".into(),
        format!("{} просить «{}» (купив {})", request.requester_name, request.nft_title, request.bought_on), &request.nft_title).await;
    Ok((StatusCode::CREATED, Json(request)))
}

/// `GET /api/gift-requests` — requests for my NFTs and requests I made.
pub async fn mine(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<MyRequests>> {
    let (incoming, outgoing) = tokio::join!(query(&state, "ownerId", &auth.uid), query(&state, "requesterId", &auth.uid));
    Ok(Json(MyRequests { incoming: incoming?, outgoing: outgoing? }))
}

/// Index of the NFT to give for a request: the requested one when it is free,
/// else a free copy of the same collection (not in a gift link, not on sale).
fn pick_copy(wallet: &[crate::models::Nft], requested: &str, reserved: &[String]) -> Option<usize> {
    let free = |n: &crate::models::Nft| !reserved.contains(&n.id) && !n.for_sale;
    if let Some(i) = wallet.iter().position(|n| n.id == requested && free(n)) {
        return Some(i);
    }
    let original = wallet.iter().find(|n| n.id == requested);
    let same = |n: &crate::models::Nft| match original {
        Some(o) => o.batch_id.is_some() && n.batch_id == o.batch_id || o.batch_id.is_none() && o.batch_name.is_some() && n.batch_name == o.batch_name,
        None => false,
    };
    wallet.iter().position(|n| n.gift_collection && free(n) && same(n))
}

/// `POST /api/gift-requests/:id/approve` — give the NFT to the requester.
pub async fn approve(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<GiftRequest>> {
    let _guard = CLAIM_LOCK.lock().await;
    let mut request = load(&state, &id).await?;
    if request.owner_id != auth.uid {
        return Err(AppError::Forbidden("Не ваш запит".into()));
    }
    if request.status != "pending" {
        return Err(AppError::Conflict("На цей запит уже відповіли".into()));
    }
    let reserved = reserved_ids(&state, &auth.uid).await?;
    let mut owner_wallet = load_nfts(&state, &auth.uid).await?;
    // Copies of a collection are interchangeable: if the scanned one waits in a
    // gift link (or is gone), give any free copy of the same collection instead.
    let pos = pick_copy(&owner_wallet, &request.nft_id, &reserved)
        .ok_or_else(|| AppError::Conflict("Усі NFT цієї колекції вже в посиланнях-подарунках або передані — скасуйте одне посилання".into()))?;
    let source = owner_wallet.remove(pos);
    request.nft_id = source.id.clone();
    let owner = user_doc(&state, &auth.uid).await?;
    let owner_name = Some(text(&owner, "companyName")).filter(|n| !n.is_empty()).unwrap_or_else(|| text(&owner, "name"));
    let now = Utc::now().to_rfc3339();
    let username = (!request.requester_username.is_empty()).then(|| request.requester_username.clone());
    let new_id = Uuid::new_v4().to_string();
    let mint = source.mint_address.clone();
    let given = crate::models::Nft {
        owners: source.owners_with(&request.requester_id, &request.requester_name, username, "gift", &now),
        id: new_id.clone(),
        owner_id: request.requester_id.clone(),
        owner_name: request.requester_name.clone(),
        for_sale: false,
        price: None,
        gift_from: Some(owner_name.clone()),
        gift_id: Some(format!("request:{}", request.id)),
        purchase: Some(crate::models::PurchaseInfo { date: request.bought_on.clone(), place: request.bought_at.clone() }),
        // Now the requester's own item: nobody can ask them for it as a gift.
        gift_collection: false,
        ..source
    };
    save_nfts(&state, &auth.uid, &owner_wallet).await?;
    let mut their_wallet = load_nfts(&state, &request.requester_id).await?;
    their_wallet.push(given);
    save_nfts(&state, &request.requester_id, &their_wallet).await?;
    crate::handlers::nfts::record_move(&state, &request.nft_id, &request.requester_id, &new_id).await;
    crate::handlers::onchain::spawn_custody_move(state.clone(), auth.uid.clone(), request.requester_id.clone(), mint.into_iter().collect());

    request.status = "approved".into();
    request.decided_at = Some(now.clone());
    request.given_nft_id = Some(new_id);
    save(&state, &request).await?;

    // Other people asking for the same NFT get a clear answer.
    for mut other in query(&state, "ownerId", &auth.uid).await? {
        if other.nft_id == request.nft_id && other.status == "pending" && other.id != request.id {
            other.status = "declined".into();
            other.decided_at = Some(now.clone());
            let _ = save(&state, &other).await;
        }
    }

    let (st, uid, title) = (state.clone(), request.requester_id.clone(), request.nft_title.clone());
    tokio::spawn(async move {
        notify(&st, &uid, "Подарунок видано 🎁".into(),
            format!("«{}» від {} уже у ваших паспортах", title, owner_name), &title).await;
    });
    Ok(Json(request))
}

/// `POST /api/gift-requests/:id/decline`
pub async fn decline(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<GiftRequest>> {
    let mut request = load(&state, &id).await?;
    if request.owner_id != auth.uid {
        return Err(AppError::Forbidden("Не ваш запит".into()));
    }
    if request.status != "pending" {
        return Err(AppError::Conflict("На цей запит уже відповіли".into()));
    }
    request.status = "declined".into();
    request.decided_at = Some(Utc::now().to_rfc3339());
    save(&state, &request).await?;
    notify(&state, &request.requester_id, "Запит на подарунок відхилено".into(),
        format!("Власник не видав «{}»", request.nft_title), &request.nft_title).await;
    Ok(Json(request))
}

#[cfg(test)]
mod tests {
    use super::pick_copy;

    fn nft(id: &str, batch: &str) -> crate::models::Nft {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": "MARKI IDENTITY", "description": "", "image": "", "category": null,
            "blockchain": "icp", "royalty": 0.0, "ownerId": "o", "ownerName": "o", "price": null,
            "currency": null, "createdAt": "", "batchId": batch, "giftCollection": true
        })).unwrap()
    }

    #[test]
    fn a_reserved_copy_is_swapped_for_a_free_one() {
        let w = vec![nft("a", "b1"), nft("b", "b1"), nft("x", "b2")];
        assert_eq!(pick_copy(&w, "a", &[]), Some(0));
        // "a" waits in a gift link: give "b" from the same collection.
        assert_eq!(pick_copy(&w, "a", &["a".into()]), Some(1));
        // Nothing free in that collection (the other one isn't a copy).
        assert_eq!(pick_copy(&w, "a", &["a".into(), "b".into()]), None);
    }
}
