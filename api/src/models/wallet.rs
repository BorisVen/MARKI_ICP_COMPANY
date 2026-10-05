use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WalletType {
    Phantom,
}

/// Stored in `wallets/{uid}`.
/// The `password` field that exists in legacy docs is intentionally absent —
/// serde silently ignores unknown fields on deserialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkiWallet {
    pub address: String,
    pub recovery_phrase: String,
    /// `{ "ICP": 0.0, "POLYGON": 0.0, "SOLANA": 0.0 }`
    #[serde(default)]
    pub balance: HashMap<String, f64>,
    #[serde(default)]
    pub fingerprint_enabled: bool,
}

/// API-safe view of MarkiWallet — recovery phrase is included because the
/// authenticated owner is allowed to see it (WalletSettings screen).
/// The legacy `password` field is never exposed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkiWalletView {
    pub address: String,
    pub recovery_phrase: String,
    pub balance: HashMap<String, f64>,
    pub fingerprint_enabled: bool,
}

impl From<MarkiWallet> for MarkiWalletView {
    fn from(w: MarkiWallet) -> Self {
        MarkiWalletView {
            address: w.address,
            recovery_phrase: w.recovery_phrase,
            balance: w.balance,
            fingerprint_enabled: w.fingerprint_enabled,
        }
    }
}

/// Stored in `crypto_wallets/{uid}` as the `wallets` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoWallet {
    pub id: String,
    #[serde(rename = "type")]
    pub wallet_type: WalletType,
    pub address: String,
    pub network: String,
    pub balance: f64,
    pub currency: String,
    pub is_connected: bool,
    pub connected_at: String,
    pub last_used: String,
    pub label: Option<String>,
}

// ── Request bodies ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCryptoWalletRequest {
    pub address: String,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFingerprintRequest {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEmailRequest {
    pub new_email: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuyNftRequest {
    pub post_id: String,
    pub buyer_wallet_id: String,
    pub nft_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodOrderRequest {
    pub post_id: String,
    pub nft_id: String,
    pub delivery_address: String,
    pub currency: String,
    /// Recipient's full name as they want it to appear on the parcel.
    pub full_name: String,
    /// Phone number for courier / Nova Poshta SMS.
    pub phone: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodOrder {
    pub id: String,
    pub post_id: String,
    pub nft_id: String,
    pub buyer_id: String,
    pub buyer_name: String,
    pub seller_id: String,
    pub nft_title: String,
    pub price: f64,
    pub nft_currency: String,
    pub payment_currency: String,
    pub delivery_address: String,
    pub full_name: String,
    pub phone: String,
    /// `pending | in_delivery | completed | cancelled`.
    pub status: String,
    pub created_at: String,
    /// Set once an owner accepts the order and a Delivery row is created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptCodOrderRequest {
    pub carrier_type: String,
    pub np_tracking_number: Option<String>,
    pub courier_id: Option<String>,
    pub controller_id: Option<String>,
    pub nfc_uid: Option<String>,
}
