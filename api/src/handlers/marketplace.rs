use axum::{extract::State, http::StatusCode, Extension, Json};
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    models::{BuyNftRequest, CodOrder, CodOrderRequest, CryptoWallet, Nft},
    notification_helpers,
    AppState,
};

/// `POST /api/marketplace/buy`
///
/// Executes a full NFT purchase. Steps (all sequential, best-effort):
///  1. Fetch and validate the marketplace post
///  2. Fetch the seller's NFT from their wallet
///  3. Fetch the buyer's Phantom wallet and verify sufficient balance
///  4. Create the NFT in the buyer's `marki_wallets` array
///  5. Deduct price + 1% fee from the buyer's crypto wallet balance
///  6. Credit the seller's Marki wallet balance
///  7. Remove the NFT from the seller's `marki_wallets` array
///  8. Delete the marketplace post
///  9. Send purchase / sale notifications
pub async fn buy_nft(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(body): Json<BuyNftRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    // ── 1. Fetch post ─────────────────────────────────────────────────────────
    let post_doc = state
        .firestore
        .get("posts", &body.post_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Listing not found".to_owned()))?;

    let for_sale = post_doc
        .get("forSale")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !for_sale {
        return Err(AppError::BadRequest("This NFT is not for sale".to_owned()));
    }

    let price = post_doc
        .get("price")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| AppError::BadRequest("Listing has no price".to_owned()))?;

    let currency = post_doc
        .get("currency")
        .and_then(|v| v.as_str())
        .unwrap_or("SOL")
        .to_owned();

    let seller_id = post_doc
        .get("userId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Post missing userId")))?
        .to_owned();

    let nft_title = post_doc
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("NFT")
        .to_owned();

    if seller_id == auth.uid {
        return Err(AppError::BadRequest("Cannot buy your own NFT".to_owned()));
    }

    // ── 2. Fetch seller's NFT ─────────────────────────────────────────────────
    let seller_nfts = load_nfts_for_user(&state, &seller_id).await?;
    let seller_nft = seller_nfts
        .iter()
        .find(|n| n.id == body.nft_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound("Seller's NFT not found".to_owned()))?;

    // ── 3. Fetch buyer's wallet and check balance ─────────────────────────────
    let mut buyer_wallets = load_crypto_wallets_for_user(&state, &auth.uid).await?;
    let buyer_wallet = buyer_wallets
        .iter_mut()
        .find(|w| w.id == body.buyer_wallet_id)
        .ok_or_else(|| AppError::NotFound("Buyer wallet not found".to_owned()))?;

    if buyer_wallet.currency.to_uppercase() != currency.to_uppercase() {
        return Err(AppError::BadRequest(format!(
            "Buyer wallet is {} but NFT is priced in {}",
            buyer_wallet.currency, currency
        )));
    }

    let fee = price * 0.01;
    let total_cost = price + fee;
    if buyer_wallet.balance < total_cost {
        return Err(AppError::BadRequest(format!(
            "Insufficient balance: need {:.6} {}, have {:.6} {}",
            total_cost, currency, buyer_wallet.balance, currency
        )));
    }

    // ── 4. Add NFT to buyer's wallet ──────────────────────────────────────────
    // Fetch buyer's name for notifications.
    let buyer_doc = state.firestore.get("users", &auth.uid).await.ok().flatten();
    let buyer_name = buyer_doc
        .as_ref()
        .and_then(|d| d.get("name").and_then(|v| v.as_str()))
        .unwrap_or("Unknown")
        .to_owned();
    let _buyer_avatar = buyer_doc
        .as_ref()
        .and_then(|d| d.get("avatar").and_then(|v| v.as_str()))
        .map(str::to_owned);

    let new_nft_id = uuid::Uuid::new_v4().to_string();
    let buyer_nft = Nft {
        id: new_nft_id.clone(),
        owner_id: auth.uid.clone(),
        owner_name: buyer_name.clone(),
        for_sale: false,
        price: None,
        ..seller_nft.clone()
    };

    let mut buyer_nfts = load_nfts_for_user(&state, &auth.uid).await?;
    buyer_nfts.push(buyer_nft);
    save_nfts_for_user(&state, &auth.uid, &buyer_nfts).await?;

    // ── 5. Deduct buyer's balance ─────────────────────────────────────────────
    buyer_wallet.balance -= total_cost;
    save_crypto_wallets_for_user(&state, &auth.uid, &buyer_wallets).await?;

    // ── 6. Credit seller's Marki wallet ───────────────────────────────────────
    if let Ok(Some(wallet_doc)) = state.firestore.get("wallets", &seller_id).await {
        let mut balance: std::collections::HashMap<String, f64> = wallet_doc
            .get("balance")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let entry = balance.entry(currency.to_uppercase()).or_insert(0.0);
        *entry += price;

        let _ = state
            .firestore
            .update("wallets", &seller_id, &json!({ "balance": balance }))
            .await;
    }

    // ── 7. Remove NFT from seller's wallet ────────────────────────────────────
    let updated_seller_nfts: Vec<Nft> = seller_nfts
        .into_iter()
        .filter(|n| n.id != body.nft_id)
        .collect();
    save_nfts_for_user(&state, &seller_id, &updated_seller_nfts).await?;

    // ── 8. Delete marketplace post ────────────────────────────────────────────
    let _ = state.firestore.delete("posts", &body.post_id).await;

    // ── 9. Notifications ──────────────────────────────────────────────────────
    notification_helpers::notify_purchase(
        &state.firestore,
        &auth.uid,
        &nft_title,
        price,
        &currency,
    )
    .await;
    notification_helpers::notify_sale(
        &state.firestore,
        &seller_id,
        &nft_title,
        price,
        &currency,
        &buyer_name,
    )
    .await;

    Ok((
        StatusCode::OK,
        Json(json!({
            "success": true,
            "nftId": new_nft_id,
            "paid": total_cost,
            "currency": currency,
        })),
    ))
}

/// `POST /api/marketplace/cod`
///
/// Places a Cash-on-Delivery order via Nova Poshta.
/// Does NOT execute a blockchain transaction — ownership stays with seller
/// until delivery is confirmed. Notifies the seller with the buyer's address.
pub async fn buy_nft_cod(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(body): Json<CodOrderRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    // ── 1. Fetch and validate marketplace post ────────────────────────────────
    let post_doc = state
        .firestore
        .get("posts", &body.post_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Listing not found".to_owned()))?;

    let for_sale = post_doc
        .get("forSale")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !for_sale {
        return Err(AppError::BadRequest("This NFT is not for sale".to_owned()));
    }

    let price = post_doc
        .get("price")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| AppError::BadRequest("Listing has no price".to_owned()))?;

    let nft_currency = post_doc
        .get("currency")
        .and_then(|v| v.as_str())
        .unwrap_or("SOL")
        .to_owned();

    let seller_id = post_doc
        .get("userId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Post missing userId")))?
        .to_owned();

    let nft_title = post_doc
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("NFT")
        .to_owned();

    if seller_id == auth.uid {
        return Err(AppError::BadRequest("Cannot buy your own NFT".to_owned()));
    }

    // ── 2. Fetch buyer name ───────────────────────────────────────────────────
    let buyer_doc  = state.firestore.get("users", &auth.uid).await.ok().flatten();
    let buyer_name = buyer_doc
        .as_ref()
        .and_then(|d| d.get("name").and_then(|v| v.as_str()))
        .unwrap_or("Unknown")
        .to_owned();

    if body.full_name.trim().is_empty() {
        return Err(AppError::BadRequest("Full name is required".to_owned()));
    }
    if body.phone.trim().is_empty() {
        return Err(AppError::BadRequest("Phone number is required".to_owned()));
    }

    // ── 3. Record COD order ───────────────────────────────────────────────────
    let order_id = uuid::Uuid::new_v4().to_string();
    let order = CodOrder {
        id:               order_id.clone(),
        post_id:          body.post_id.clone(),
        nft_id:           body.nft_id.clone(),
        buyer_id:         auth.uid.clone(),
        buyer_name:       buyer_name.clone(),
        seller_id:        seller_id.clone(),
        nft_title:        nft_title.clone(),
        price,
        nft_currency:     nft_currency.clone(),
        payment_currency: body.currency.clone(),
        delivery_address: body.delivery_address.clone(),
        full_name:        body.full_name.clone(),
        phone:            body.phone.clone(),
        status:           "pending".to_owned(),
        created_at:       Utc::now().to_rfc3339(),
        delivery_id:      None,
    };
    let order_value = serde_json::to_value(&order)
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
    state
        .firestore
        .set("cod_orders", &order_id, &order_value)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    // ── 4. Notifications ──────────────────────────────────────────────────────
    notification_helpers::notify_cod_buyer(
        &state.firestore,
        &auth.uid,
        &nft_title,
        price,
        &nft_currency,
        &body.delivery_address,
    )
    .await;
    notification_helpers::notify_cod_seller(
        &state.firestore,
        &seller_id,
        &order_id,
        &nft_title,
        price,
        &nft_currency,
        &buyer_name,
        &body.full_name,
        &body.phone,
        &body.delivery_address,
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "success": true,
            "orderId": order_id,
            "status": "pending",
        })),
    ))
}

// ── Private helpers ───────────────────────────────────────────────────────────

async fn load_nfts_for_user(state: &AppState, uid: &str) -> ApiResult<Vec<Nft>> {
    let doc = state
        .firestore
        .get("marki_wallets", uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({ "nfts": [] }));

    Ok(doc
        .get("nfts")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

async fn save_nfts_for_user(state: &AppState, uid: &str, nfts: &[Nft]) -> ApiResult<()> {
    state
        .firestore
        .update("marki_wallets", uid, &json!({ "nfts": nfts }))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))
}

async fn load_crypto_wallets_for_user(
    state: &AppState,
    uid: &str,
) -> ApiResult<Vec<CryptoWallet>> {
    let doc = state
        .firestore
        .get("crypto_wallets", uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({ "wallets": [] }));

    Ok(doc
        .get("wallets")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

async fn save_crypto_wallets_for_user(
    state: &AppState,
    uid: &str,
    wallets: &[CryptoWallet],
) -> ApiResult<()> {
    state
        .firestore
        .update("crypto_wallets", uid, &json!({ "wallets": wallets }))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))
}
