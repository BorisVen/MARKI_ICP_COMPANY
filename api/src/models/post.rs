use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub user_id: String,
    pub user_name: String,
    pub user_avatar: Option<String>,
    pub text: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub id: String,
    pub user_id: String,
    pub user_name: String,
    pub user_avatar: Option<String>,
    pub nft_image: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub likes: i64,
    #[serde(default)]
    pub reposts: i64,
    #[serde(default)]
    pub liked_by: Vec<String>,
    #[serde(default)]
    pub comments: Vec<Comment>,
    pub created_at: String,
    #[serde(default)]
    pub for_sale: bool,
    pub price: Option<f64>,
    pub currency: Option<String>,
    pub wallet_nft_id: Option<String>,
    /// Collection posts: all image URLs. Empty on single-NFT posts.
    #[serde(default)]
    pub nft_images: Vec<String>,
    /// Collection posts: all wallet NFT IDs. Empty on single-NFT posts.
    #[serde(default)]
    pub wallet_nft_ids: Vec<String>,
    /// Seller's primary Phantom wallet address — set at listing time so the
    /// buyer's frontend knows where to send SOL without an extra round-trip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seller_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePostRequest {
    /// Primary image. Optional when `nft_images` is supplied (first element is used as fallback).
    pub nft_image: Option<String>,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub for_sale: bool,
    pub price: Option<f64>,
    pub currency: Option<String>,
    pub wallet_nft_id: Option<String>,
    /// Collection posts only: all image URLs.
    #[serde(default)]
    pub nft_images: Vec<String>,
    /// Collection posts only: all wallet NFT IDs.
    #[serde(default)]
    pub wallet_nft_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddCommentRequest {
    pub text: String,
}
