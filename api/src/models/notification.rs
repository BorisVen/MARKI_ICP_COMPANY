use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationType {
    Purchase,
    Sale,
    Like,
    Comment,
    Welcome,
    Wallet,
    NftCreated,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NotificationMeta {
    pub nft_id: Option<String>,
    pub nft_title: Option<String>,
    pub price: Option<f64>,
    pub currency: Option<String>,
    pub from_user: Option<String>,
    pub batch_count: Option<usize>,
    // ── COD-specific (let alerts deep-link into CRM) ──────────────────────────
    pub order_id: Option<String>,
    pub delivery_id: Option<String>,
    pub delivery_address: Option<String>,
    pub buyer_full_name: Option<String>,
    pub buyer_phone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: String,
    pub user_id: String,
    #[serde(rename = "type")]
    pub notification_type: NotificationType,
    pub title: String,
    pub text: String,
    #[serde(default)]
    pub read: bool,
    pub created_at: String,
    pub metadata: Option<NotificationMeta>,
}
