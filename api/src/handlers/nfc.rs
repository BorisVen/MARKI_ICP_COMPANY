use axum::{extract::State, http::StatusCode, Extension, Json};
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    models::{BindNfcRequest, Delivery, Nft, VerifyNfcRequest, VerifyNfcResponse},
    services::firestore::QueryFilter,
    AppState,
};

/// Strip whitespace, colons, dashes; uppercase. NTAG UIDs come in many forms
/// from different readers ("04:A1:B2:..." vs "04A1B2..."); we normalise once
/// here so binding and verification always compare apples to apples.
fn normalize_uid(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_uppercase()
}

async fn load_nfts_for_user(state: &AppState, uid: &str) -> ApiResult<Vec<Nft>> {
    let doc = state.firestore.get("marki_wallets", uid).await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({ "nfts": [] }));
    Ok(doc.get("nfts")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

async fn save_nfts_for_user(state: &AppState, uid: &str, nfts: &[Nft]) -> ApiResult<()> {
    state.firestore.update("marki_wallets", uid, &json!({ "nfts": nfts })).await
        .map_err(|e| AppError::Firebase(e.to_string()))
}

/// `POST /api/nfc/bind`
///
/// Stores the physical NFC tag's UID alongside the NFT record, plus a top-level
/// `nfc_bindings/{uid}` lookup document so verification is a single read.
///
/// MVP-level security: we trust the UID at binding time (NTAG 216 has no
/// crypto). For production, swap to NTAG 424 DNA + CMAC verification —
/// the binding record then stores the chip's diversified key reference instead
/// of (or in addition to) the UID.
pub async fn bind_nfc(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(body): Json<BindNfcRequest>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let uid_norm = normalize_uid(&body.nfc_uid);
    if uid_norm.is_empty() {
        return Err(AppError::BadRequest("nfcUid is empty".to_owned()));
    }

    let mut nfts = load_nfts_for_user(&state, &auth.uid).await?;
    let nft = nfts.iter_mut().find(|n| n.id == body.nft_id)
        .ok_or_else(|| AppError::NotFound(
            "NFT not found in your wallet".to_owned(),
        ))?;

    // Reject if this UID is already bound to a different NFT.
    if let Ok(Some(existing)) = state.firestore.get("nfc_bindings", &uid_norm).await {
        let existing_nft = existing.get("nftId").and_then(|v| v.as_str()).unwrap_or("");
        if !existing_nft.is_empty() && existing_nft != body.nft_id {
            return Err(AppError::BadRequest(
                "This NFC chip is already bound to a different NFT".to_owned(),
            ));
        }
    }

    let now = Utc::now().to_rfc3339();
    let binding = json!({
        "nfcUid":  uid_norm,
        "nftId":   nft.id,
        "ownerId": auth.uid,
        "ownerName": nft.owner_name,
        "boundAt": now,
    });
    state.firestore.set("nfc_bindings", &uid_norm, &binding).await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    // Also stamp the binding into the NFT record so it travels with it.
    // We don't have a public field on Nft for this yet; storing it in the
    // wallet doc as a parallel `nfcBindings` map keeps the schema additive.
    let nft_id = nft.id.clone();
    save_nfts_for_user(&state, &auth.uid, &nfts).await?;

    let mut bindings: serde_json::Map<String, serde_json::Value> = state.firestore
        .get("marki_wallets", &auth.uid).await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .as_ref()
        .and_then(|d| d.get("nfcBindings").and_then(|v| v.as_object()).cloned())
        .unwrap_or_default();
    bindings.insert(nft_id.clone(), json!(uid_norm));
    state.firestore.update("marki_wallets", &auth.uid,
        &json!({ "nfcBindings": serde_json::Value::Object(bindings) })).await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(json!({
        "success": true,
        "nfcUid": uid_norm,
        "nftId": nft_id,
    }))))
}

/// `POST /api/nfc/verify`
///
/// Looks up an NFC UID and returns the bound NFT. If the caller is the buyer
/// of an active delivery for this NFT, the delivery is auto-confirmed as
/// received (this is the "tap → done" UX described in the build plan).
pub async fn verify_nfc(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(body): Json<VerifyNfcRequest>,
) -> ApiResult<Json<VerifyNfcResponse>> {
    let uid_norm = normalize_uid(&body.nfc_uid);
    let binding = state.firestore.get("nfc_bindings", &uid_norm).await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(
            "No NFT is bound to this NFC tag".to_owned(),
        ))?;

    let nft_id = binding.get("nftId").and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Binding missing nftId")))?
        .to_owned();
    let owner_id = binding.get("ownerId").and_then(|v| v.as_str()).unwrap_or("").to_owned();

    let owner_nfts = load_nfts_for_user(&state, &owner_id).await?;
    let nft = owner_nfts.into_iter().find(|n| n.id == nft_id)
        .ok_or_else(|| AppError::NotFound(
            "NFT no longer exists in owner's wallet".to_owned(),
        ))?;

    // Look for an active delivery of this NFT addressed to the caller.
    let mut auto_confirmed = false;
    let mut delivery_id: Option<String> = None;

    let docs = state.firestore.query(
        "deliveries",
        vec![
            QueryFilter::equal("nftId", nft_id.clone()),
            QueryFilter::equal("buyerId", auth.uid.clone()),
        ],
        None, None,
    ).await.unwrap_or_default();

    for d in docs {
        let mut delivery: Delivery = match serde_json::from_value(d) {
            Ok(x) => x,
            Err(_) => continue,
        };
        if delivery.customer_received { continue; }

        let now = Utc::now().to_rfc3339();
        delivery.customer_received = true;
        delivery.received_at = Some(now.clone());
        delivery.nfc_verified = true;
        delivery.nfc_verified_at = Some(now.clone());
        delivery.status = "verified".to_owned();
        delivery.checkpoints.push(crate::models::DeliveryCheckpoint {
            id: uuid::Uuid::new_v4().to_string(),
            status: "Customer verified via NFC".to_owned(),
            location: delivery.delivery_address.clone(),
            timestamp: now,
            recorded_by: auth.uid.clone(),
            recorded_by_name: None,
            note: Some(format!("UID {}", uid_norm)),
        });
        delivery.updated_at = Utc::now().to_rfc3339();

        let val = serde_json::to_value(&delivery).map_err(|e| AppError::Internal(e.into()))?;
        let _ = state.firestore.set("deliveries", &delivery.id, &val).await;
        if let Some(order_id) = &delivery.order_id {
            let _ = state.firestore.update(
                "cod_orders", order_id,
                &json!({ "status": "completed" }),
            ).await;
        }
        delivery_id = Some(delivery.id.clone());
        auto_confirmed = true;
        break;
    }

    Ok(Json(VerifyNfcResponse {
        nft_id,
        nft_title: nft.title,
        owner_id: nft.owner_id,
        owner_name: nft.owner_name,
        mint_address: nft.mint_address,
        delivery_id,
        auto_confirmed_receipt: auto_confirmed,
    }))
}
