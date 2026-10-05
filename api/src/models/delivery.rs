use serde::{Deserialize, Serialize};

/// One step in a delivery's lifecycle. Mirrors how Nova Poshta exposes movement
/// history: a list of timestamped checkpoints with a free-form status string
/// and a location (warehouse, city, hub). For self-delivery the courier writes
/// these manually; for Nova Poshta they're synced from their API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryCheckpoint {
    pub id: String,
    pub status: String,
    pub location: String,
    pub timestamp: String,
    /// uid of the user who logged this checkpoint, or `"nova_poshta"` for auto-synced rows.
    pub recorded_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_by_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub id: String,
    /// Optional source COD order this delivery was created from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
    pub nft_id: String,
    pub nft_title: String,
    pub seller_id: String,
    pub buyer_id: String,
    pub buyer_name: String,
    pub delivery_address: String,

    /// `"self"` (the business delivers in-house) or `"nova_poshta"`.
    pub carrier_type: String,

    // ── Self carrier ──────────────────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub courier_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub courier_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controller_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controller_name: Option<String>,

    // ── Nova Poshta carrier ───────────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub np_tracking_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub np_last_synced_at: Option<String>,

    /// `pending | assigned | picked_up | in_transit | out_for_delivery | delivered | verified | failed`.
    pub status: String,

    #[serde(default)]
    pub checkpoints: Vec<DeliveryCheckpoint>,

    /// Set to `true` once the customer either presses "I received it" or scans
    /// the NFC tag bound to the NFT.
    #[serde(default)]
    pub customer_received: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received_at: Option<String>,

    // ── NFC binding (MVP: NTAG 216 — UID match only) ──────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nfc_uid: Option<String>,
    #[serde(default)]
    pub nfc_verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nfc_verified_at: Option<String>,

    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDeliveryRequest {
    /// If present, COD order to mark as "in delivery". Optional — owners can
    /// also create deliveries manually for off-platform sales.
    pub order_id: Option<String>,
    pub nft_id: String,
    pub buyer_id: String,
    pub delivery_address: String,
    pub carrier_type: String,
    pub np_tracking_number: Option<String>,
    pub courier_id: Option<String>,
    pub controller_id: Option<String>,
    pub nfc_uid: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCarrierRequest {
    pub carrier_type: String,
    pub np_tracking_number: Option<String>,
    pub courier_id: Option<String>,
    pub controller_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCheckpointRequest {
    pub status: String,
    pub location: String,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatusRequest {
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindNfcRequest {
    pub nft_id: String,
    pub nfc_uid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyNfcRequest {
    pub nfc_uid: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyNfcResponse {
    pub nft_id: String,
    pub nft_title: String,
    pub owner_id: String,
    pub owner_name: String,
    pub mint_address: Option<String>,
    /// If the buyer is scanning their own pending delivery, the matching record.
    pub delivery_id: Option<String>,
    /// Whether this scan auto-confirmed receipt of an active delivery.
    pub auto_confirmed_receipt: bool,
}
