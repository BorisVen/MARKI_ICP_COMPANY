use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    models::{
        AcceptCodOrderRequest, CodOrder, Delivery, DeliveryCheckpoint, UserData,
    },
    services::firestore::QueryFilter,
    AppState,
};

const VALID_CARRIERS: &[&str] = &["self", "nova_poshta"];

async fn fetch_user(state: &AppState, uid: &str) -> ApiResult<Option<UserData>> {
    let doc = state.firestore.get("users", uid).await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(doc.and_then(|v| serde_json::from_value(v).ok()))
}

/// `GET /api/cod-orders`
///
/// Lists COD orders the caller can act on:
/// - sellers see their own pending orders (this is the CRM "inbox").
/// - buyers see their own orders (so they can track status).
pub async fn list_orders(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<CodOrder>>> {
    let mut docs = state.firestore.query(
        "cod_orders",
        vec![QueryFilter::equal("sellerId", auth.uid.clone())],
        None, None,
    ).await.map_err(|e| AppError::Firebase(e.to_string()))?;

    let mut by_buyer = state.firestore.query(
        "cod_orders",
        vec![QueryFilter::equal("buyerId", auth.uid.clone())],
        None, None,
    ).await.map_err(|e| AppError::Firebase(e.to_string()))?;
    docs.append(&mut by_buyer);

    // If the caller is staff of a company, surface the company's orders too.
    if let Some(me) = fetch_user(&state, &auth.uid).await? {
        if let Some(cid) = me.company_id {
            if cid != auth.uid {
                let mut by_company = state.firestore.query(
                    "cod_orders",
                    vec![QueryFilter::equal("sellerId", cid)],
                    None, None,
                ).await.map_err(|e| AppError::Firebase(e.to_string()))?;
                docs.append(&mut by_company);
            }
        }
    }

    let mut out: Vec<CodOrder> = docs.into_iter()
        .filter_map(|d| serde_json::from_value(d).ok())
        .collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out.dedup_by(|a, b| a.id == b.id);

    Ok(Json(out))
}

/// `POST /api/cod-orders/:id/accept`
///
/// Seller (or company staff) takes a pending COD order and converts it into a
/// `Delivery` record with all buyer details auto-filled — no manual UID lookup.
pub async fn accept_order(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(order_id): Path<String>,
    Json(body): Json<AcceptCodOrderRequest>,
) -> ApiResult<(StatusCode, Json<Delivery>)> {
    if !VALID_CARRIERS.contains(&body.carrier_type.as_str()) {
        return Err(AppError::BadRequest(format!(
            "carrierType must be one of {:?}", VALID_CARRIERS
        )));
    }

    let order_doc = state.firestore.get("cod_orders", &order_id).await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Order not found".to_owned()))?;
    let order: CodOrder = serde_json::from_value(order_doc)
        .map_err(|e| AppError::Internal(e.into()))?;

    // Authorisation: the seller, or company staff working for the seller.
    let mut allowed = order.seller_id == auth.uid;
    if !allowed {
        if let Some(me) = fetch_user(&state, &auth.uid).await? {
            if me.company_id.as_deref() == Some(order.seller_id.as_str()) {
                allowed = true;
            }
        }
    }
    if !allowed {
        return Err(AppError::Forbidden(
            "Only the seller or their company can accept this order".to_owned(),
        ));
    }

    if order.status != "pending" {
        return Err(AppError::BadRequest(format!(
            "Order is not pending (status = {})", order.status
        )));
    }

    let courier = match &body.courier_id {
        Some(cid) => fetch_user(&state, cid).await?,
        None => None,
    };
    let controller = match &body.controller_id {
        Some(cid) => fetch_user(&state, cid).await?,
        None => None,
    };
    let me_name = fetch_user(&state, &auth.uid).await?
        .map(|u| u.name).unwrap_or_default();

    let now = Utc::now().to_rfc3339();
    let delivery_id = uuid::Uuid::new_v4().to_string();

    let initial_status = if body.carrier_type == "self" && courier.is_some() {
        "assigned"
    } else {
        "pending"
    };

    let delivery = Delivery {
        id: delivery_id.clone(),
        order_id: Some(order.id.clone()),
        nft_id: order.nft_id.clone(),
        nft_title: order.nft_title.clone(),
        seller_id: order.seller_id.clone(),
        buyer_id: order.buyer_id.clone(),
        // Prefer the recipient name on the parcel; fall back to account name.
        buyer_name: if order.full_name.is_empty() { order.buyer_name.clone() } else { order.full_name.clone() },
        delivery_address: order.delivery_address.clone(),
        carrier_type: body.carrier_type,
        courier_id: body.courier_id.clone(),
        courier_name: courier.as_ref().map(|u| u.name.clone()),
        controller_id: body.controller_id.clone(),
        controller_name: controller.as_ref().map(|u| u.name.clone()),
        np_tracking_number: body.np_tracking_number.clone(),
        np_last_synced_at: None,
        status: initial_status.to_owned(),
        checkpoints: vec![DeliveryCheckpoint {
            id: uuid::Uuid::new_v4().to_string(),
            status: format!("Order accepted by {}", me_name),
            location: "—".to_owned(),
            timestamp: now.clone(),
            recorded_by: auth.uid.clone(),
            recorded_by_name: Some(me_name.clone()),
            note: Some(format!(
                "Recipient: {} · Phone: {} · {}",
                order.full_name, order.phone, order.delivery_address
            )),
        }],
        customer_received: false,
        received_at: None,
        nfc_uid: body.nfc_uid.clone(),
        nfc_verified: false,
        nfc_verified_at: None,
        created_at: now.clone(),
        updated_at: now,
    };

    let val = serde_json::to_value(&delivery).map_err(|e| AppError::Internal(e.into()))?;
    state.firestore.set("deliveries", &delivery_id, &val).await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    state.firestore.update("cod_orders", &order.id,
        &json!({ "status": "in_delivery", "deliveryId": delivery_id })).await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(delivery)))
}
