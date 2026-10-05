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
        AddCheckpointRequest, CreateDeliveryRequest, Delivery, DeliveryCheckpoint,
        UpdateCarrierRequest, UpdateStatusRequest, UserData,
    },
    services::firestore::QueryFilter,
    AppState,
};

const VALID_STATUSES: &[&str] = &[
    "pending", "assigned", "picked_up", "in_transit",
    "out_for_delivery", "delivered", "verified", "failed",
];

const VALID_CARRIERS: &[&str] = &["self", "nova_poshta"];

async fn fetch_user(state: &AppState, uid: &str) -> ApiResult<Option<UserData>> {
    let doc = state.firestore.get("users", uid).await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(doc.and_then(|v| serde_json::from_value(v).ok()))
}

fn name_from(user: &Option<UserData>) -> Option<String> {
    user.as_ref().map(|u| u.name.clone())
}

/// `GET /api/deliveries`
///
/// Returns deliveries scoped by the caller's role:
/// - **owner / manager / controller** see deliveries where `seller_id == auth.uid`
///   OR the seller's `company_id` matches theirs.
/// - **courier** sees deliveries assigned to them.
/// - everyone else (customer) sees deliveries where they're the buyer.
pub async fn list_deliveries(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<Delivery>>> {
    let me = fetch_user(&state, &auth.uid).await?
        .ok_or_else(|| AppError::NotFound("User profile not found".to_owned()))?;

    let has_role = |r: &str| me.roles.iter().any(|x| x == r);

    // Three independent scopes — we always run all of them and merge:
    //   1. seller scope (the caller is the NFT owner who sold it)
    //   2. company scope (manager/controller working for an owner)
    //   3. courier scope (assigned courier)
    //   4. buyer scope (the caller is the recipient)
    // Earlier this was gated behind `roles[]`, which meant a plain
    // company-approved owner with empty roles got zero results — that's
    // why deliveries "disappeared" after acceptance. Fix: always include
    // sellerId == auth.uid, regardless of role array contents.
    let mut docs = Vec::new();

    let mut by_seller = state.firestore.query(
        "deliveries",
        vec![QueryFilter::equal("sellerId", auth.uid.clone())],
        None, None,
    ).await.map_err(|e| AppError::Firebase(e.to_string()))?;
    docs.append(&mut by_seller);

    if has_role("manager") || has_role("controller") {
        if let Some(cid) = me.company_id.as_deref() {
            if cid != auth.uid {
                let mut by_company = state.firestore.query(
                    "deliveries",
                    vec![QueryFilter::equal("sellerId", cid.to_string())],
                    None, None,
                ).await.map_err(|e| AppError::Firebase(e.to_string()))?;
                docs.append(&mut by_company);
            }
        }
    }

    if has_role("courier") {
        let mut by_courier = state.firestore.query(
            "deliveries",
            vec![QueryFilter::equal("courierId", auth.uid.clone())],
            None, None,
        ).await.map_err(|e| AppError::Firebase(e.to_string()))?;
        docs.append(&mut by_courier);
    }

    // Always include deliveries where the caller is the buyer.
    let mut by_buyer = state.firestore.query(
        "deliveries",
        vec![QueryFilter::equal("buyerId", auth.uid.clone())],
        None, None,
    ).await.map_err(|e| AppError::Firebase(e.to_string()))?;
    docs.append(&mut by_buyer);

    let mut out: Vec<Delivery> = docs.into_iter()
        .filter_map(|d| serde_json::from_value(d).ok())
        .collect();

    // De-dup by id, newest first.
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out.dedup_by(|a, b| a.id == b.id);

    Ok(Json(out))
}

/// `GET /api/deliveries/:id`
pub async fn get_delivery(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<Delivery>> {
    let d = load_delivery(&state, &id).await?;
    ensure_can_view(&state, &auth.uid, &d).await?;
    Ok(Json(d))
}

/// `POST /api/deliveries`
///
/// Creates a delivery from an existing COD order or as a standalone record.
/// Caller must be the seller (NFT owner) of the underlying NFT.
pub async fn create_delivery(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(body): Json<CreateDeliveryRequest>,
) -> ApiResult<(StatusCode, Json<Delivery>)> {
    if !VALID_CARRIERS.contains(&body.carrier_type.as_str()) {
        return Err(AppError::BadRequest(format!(
            "carrierType must be one of {:?}", VALID_CARRIERS
        )));
    }

    // Pull NFT info — we need title for display and to verify ownership.
    let me = fetch_user(&state, &auth.uid).await?
        .ok_or_else(|| AppError::NotFound("User profile not found".to_owned()))?;
    if !me.roles.iter().any(|r| r == "owner") && !me.company_approved {
        return Err(AppError::Forbidden(
            "Only company-approved owners can create deliveries".to_owned(),
        ));
    }

    let buyer = fetch_user(&state, &body.buyer_id).await?
        .ok_or_else(|| AppError::NotFound("Buyer not found".to_owned()))?;

    // Title pulled from any post mentioning this nftId is good enough; fall back to "NFT".
    let nft_title = state.firestore.query(
        "posts",
        vec![QueryFilter::equal("nftId", body.nft_id.clone())],
        None, Some(1),
    ).await.ok()
        .and_then(|docs| docs.into_iter().next())
        .and_then(|d| d.get("title").and_then(|v| v.as_str()).map(str::to_owned))
        .unwrap_or_else(|| "NFT".to_owned());

    let courier = match &body.courier_id {
        Some(cid) => fetch_user(&state, cid).await?,
        None => None,
    };
    let controller = match &body.controller_id {
        Some(cid) => fetch_user(&state, cid).await?,
        None => None,
    };

    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let initial_status = if body.carrier_type == "self" && courier.is_some() {
        "assigned"
    } else {
        "pending"
    };

    let delivery = Delivery {
        id: id.clone(),
        order_id: body.order_id.clone(),
        nft_id: body.nft_id.clone(),
        nft_title,
        seller_id: auth.uid.clone(),
        buyer_id: body.buyer_id.clone(),
        buyer_name: buyer.name.clone(),
        delivery_address: body.delivery_address.clone(),
        carrier_type: body.carrier_type.clone(),
        courier_id: body.courier_id.clone(),
        courier_name: name_from(&courier),
        controller_id: body.controller_id.clone(),
        controller_name: name_from(&controller),
        np_tracking_number: body.np_tracking_number.clone(),
        np_last_synced_at: None,
        status: initial_status.to_owned(),
        checkpoints: vec![DeliveryCheckpoint {
            id: uuid::Uuid::new_v4().to_string(),
            status: "Created".to_owned(),
            location: "—".to_owned(),
            timestamp: now.clone(),
            recorded_by: auth.uid.clone(),
            recorded_by_name: Some(me.name.clone()),
            note: None,
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
    state.firestore.set("deliveries", &id, &val).await
        .map_err(|e| AppError::Firebase(e.to_string()))?;

    // If this delivery was started from a COD order, mark the order as "in_delivery".
    if let Some(order_id) = &body.order_id {
        let _ = state.firestore.update(
            "cod_orders", order_id,
            &json!({ "status": "in_delivery", "deliveryId": id }),
        ).await;
    }

    Ok((StatusCode::CREATED, Json(delivery)))
}

/// `PUT /api/deliveries/:id/carrier`
pub async fn update_carrier(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    Json(body): Json<UpdateCarrierRequest>,
) -> ApiResult<Json<Delivery>> {
    if !VALID_CARRIERS.contains(&body.carrier_type.as_str()) {
        return Err(AppError::BadRequest("Invalid carrierType".to_owned()));
    }

    let mut d = load_delivery(&state, &id).await?;
    ensure_seller_or_company(&state, &auth.uid, &d).await?;

    d.carrier_type = body.carrier_type;
    d.np_tracking_number = body.np_tracking_number;
    d.courier_id = body.courier_id.clone();
    d.controller_id = body.controller_id.clone();

    d.courier_name = match &d.courier_id {
        Some(cid) => name_from(&fetch_user(&state, cid).await?),
        None => None,
    };
    d.controller_name = match &d.controller_id {
        Some(cid) => name_from(&fetch_user(&state, cid).await?),
        None => None,
    };

    if d.carrier_type == "self" && d.courier_id.is_some() && d.status == "pending" {
        d.status = "assigned".to_owned();
    }

    persist(&state, &d).await?;
    Ok(Json(d))
}

/// `PUT /api/deliveries/:id/status`
pub async fn update_status(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    Json(body): Json<UpdateStatusRequest>,
) -> ApiResult<Json<Delivery>> {
    if !VALID_STATUSES.contains(&body.status.as_str()) {
        return Err(AppError::BadRequest(format!(
            "status must be one of {:?}", VALID_STATUSES
        )));
    }

    let mut d = load_delivery(&state, &id).await?;
    ensure_can_modify(&state, &auth.uid, &d).await?;

    d.status = body.status;
    persist(&state, &d).await?;
    Ok(Json(d))
}

/// `POST /api/deliveries/:id/checkpoints`
pub async fn add_checkpoint(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
    Json(body): Json<AddCheckpointRequest>,
) -> ApiResult<Json<Delivery>> {
    let mut d = load_delivery(&state, &id).await?;
    ensure_can_modify(&state, &auth.uid, &d).await?;

    let me = fetch_user(&state, &auth.uid).await?;
    let cp = DeliveryCheckpoint {
        id: uuid::Uuid::new_v4().to_string(),
        status: body.status,
        location: body.location,
        timestamp: Utc::now().to_rfc3339(),
        recorded_by: auth.uid.clone(),
        recorded_by_name: name_from(&me),
        note: body.note,
    };
    d.checkpoints.push(cp);

    // Auto-advance status from `assigned` to `in_transit` on first courier checkpoint.
    if d.status == "assigned" {
        d.status = "in_transit".to_owned();
    }

    persist(&state, &d).await?;
    Ok(Json(d))
}

/// `POST /api/deliveries/:id/sync-novaposhta`
///
/// MVP stub: Nova Poshta API integration goes here. We expose the endpoint now
/// so the frontend can wire it up; implementation will call
/// `https://api.novaposhta.ua/v2.0/json/` with `getStatusDocuments` once an API
/// key is present in `Config`.
pub async fn sync_novaposhta(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<Delivery>> {
    let mut d = load_delivery(&state, &id).await?;
    ensure_can_modify(&state, &auth.uid, &d).await?;

    if d.carrier_type != "nova_poshta" {
        return Err(AppError::BadRequest(
            "Delivery is not configured for Nova Poshta".to_owned(),
        ));
    }
    let ttn = d.np_tracking_number.clone()
        .ok_or_else(|| AppError::BadRequest("Tracking number not set".to_owned()))?;

    // TODO: call Nova Poshta `getStatusDocuments`. For now, log a synthetic
    // checkpoint so the UI flow is testable end-to-end.
    d.checkpoints.push(DeliveryCheckpoint {
        id: uuid::Uuid::new_v4().to_string(),
        status: format!("NP sync placeholder for TTN {}", ttn),
        location: "Nova Poshta network".to_owned(),
        timestamp: Utc::now().to_rfc3339(),
        recorded_by: "nova_poshta".to_owned(),
        recorded_by_name: Some("Nova Poshta".to_owned()),
        note: Some("Replace with real API call once NP_API_KEY is provisioned.".to_owned()),
    });
    d.np_last_synced_at = Some(Utc::now().to_rfc3339());

    persist(&state, &d).await?;
    Ok(Json(d))
}

/// `POST /api/deliveries/:id/confirm-receipt`
///
/// Buyer confirms delivery received (manual button OR consequence of NFC scan).
pub async fn confirm_receipt(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<Delivery>> {
    let mut d = load_delivery(&state, &id).await?;
    if d.buyer_id != auth.uid {
        return Err(AppError::Forbidden(
            "Only the buyer can confirm receipt".to_owned(),
        ));
    }

    let now = Utc::now().to_rfc3339();
    d.customer_received = true;
    d.received_at = Some(now.clone());
    d.status = "delivered".to_owned();
    d.checkpoints.push(DeliveryCheckpoint {
        id: uuid::Uuid::new_v4().to_string(),
        status: "Customer confirmed receipt".to_owned(),
        location: d.delivery_address.clone(),
        timestamp: now,
        recorded_by: auth.uid.clone(),
        recorded_by_name: None,
        note: None,
    });

    persist(&state, &d).await?;

    if let Some(order_id) = &d.order_id {
        let _ = state.firestore.update(
            "cod_orders", order_id,
            &json!({ "status": "completed" }),
        ).await;
    }

    Ok(Json(d))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn load_delivery(state: &AppState, id: &str) -> ApiResult<Delivery> {
    let doc = state.firestore.get("deliveries", id).await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Delivery {} not found", id)))?;
    serde_json::from_value(doc).map_err(|e| AppError::Internal(e.into()))
}

async fn persist(state: &AppState, d: &Delivery) -> ApiResult<()> {
    let mut owned = d.clone();
    owned.updated_at = Utc::now().to_rfc3339();
    let val = serde_json::to_value(&owned).map_err(|e| AppError::Internal(e.into()))?;
    state.firestore.set("deliveries", &owned.id, &val).await
        .map_err(|e| AppError::Firebase(e.to_string()))
}

async fn ensure_can_view(state: &AppState, uid: &str, d: &Delivery) -> ApiResult<()> {
    if d.buyer_id == uid || d.seller_id == uid {
        return Ok(());
    }
    if let Some(cid) = &d.courier_id { if cid == uid { return Ok(()); } }
    if let Some(cid) = &d.controller_id { if cid == uid { return Ok(()); } }
    // Allow company staff to view deliveries owned by their company.
    if let Some(me) = fetch_user(state, uid).await? {
        if let Some(cid) = me.company_id {
            if cid == d.seller_id { return Ok(()); }
        }
    }
    Err(AppError::Forbidden("Not authorised to view this delivery".to_owned()))
}

/// Seller, courier, controller, or company staff can write checkpoints / status.
async fn ensure_can_modify(state: &AppState, uid: &str, d: &Delivery) -> ApiResult<()> {
    if d.seller_id == uid { return Ok(()); }
    if let Some(cid) = &d.courier_id { if cid == uid { return Ok(()); } }
    if let Some(cid) = &d.controller_id { if cid == uid { return Ok(()); } }
    if let Some(me) = fetch_user(state, uid).await? {
        if let Some(cid) = me.company_id {
            if cid == d.seller_id { return Ok(()); }
        }
    }
    Err(AppError::Forbidden(
        "Not authorised to modify this delivery".to_owned(),
    ))
}

async fn ensure_seller_or_company(state: &AppState, uid: &str, d: &Delivery) -> ApiResult<()> {
    if d.seller_id == uid { return Ok(()); }
    if let Some(me) = fetch_user(state, uid).await? {
        if let Some(cid) = me.company_id {
            if cid == d.seller_id { return Ok(()); }
        }
    }
    Err(AppError::Forbidden(
        "Only the seller or their company staff can change carrier".to_owned(),
    ))
}
