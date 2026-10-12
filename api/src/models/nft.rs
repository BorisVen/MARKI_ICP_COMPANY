use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Nft {
    pub id: String,
    pub title: String,
    pub description: String,
    pub image: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub category: Option<String>,
    pub blockchain: Option<String>,
    pub royalty: Option<f64>,
    pub owner_id: String,
    pub owner_name: String,
    pub price: Option<f64>,
    #[serde(default)]
    pub for_sale: bool,
    pub currency: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub views: u64,
    /// Off-chain Metaplex metadata JSON URL (set at creation time).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_uri: Option<String>,
    /// On-chain Solana mint address (set after client-side Umi mint).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mint_address: Option<String>,
    // ── Edition fields (absent on regular 1-of-1 NFTs) ────────────────────────
    /// Total print supply for a Master Edition (0 = this record IS the master).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edition_count: Option<u32>,
    /// Which print this is (0 = master, 1..N = individual prints).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edition_number: Option<u32>,
    /// ID of the master NFT record, present only on print-edition records.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_nft_id: Option<String>,
    // ── Batch/collection fields (absent on regular single NFTs) ───────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<u32>,
    /// PNG QR code (Firebase Storage URL) that opens the public NFT viewer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qr_image_url: Option<String>,
    /// Set on NFTs received as a gift: who gave it and which gift link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gift_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gift_id: Option<String>,
    /// Ownership chain, oldest first: issuer, then every gift or sale.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<OwnershipRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipRecord {
    pub uid: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub at: String,
    /// "issued", "gift" or "sale".
    pub via: String,
}

impl Nft {
    /// The ownership chain extended by a new owner. NFTs created before the chain
    /// existed start it with their current owner as the issuer.
    pub fn owners_with(&self, uid: &str, name: &str, username: Option<String>, via: &str, at: &str) -> Vec<OwnershipRecord> {
        let mut owners = self.owners.clone();
        if owners.is_empty() {
            owners.push(OwnershipRecord {
                uid: self.owner_id.clone(),
                name: self.owner_name.clone(),
                username: None,
                at: self.created_at.clone(),
                via: "issued".into(),
            });
        }
        owners.push(OwnershipRecord {
            uid: uid.to_owned(),
            name: name.to_owned(),
            username,
            at: at.to_owned(),
            via: via.to_owned(),
        });
        owners
    }

    /// Who issued the NFT: the first owner in the chain, else the current owner.
    pub fn issuer_id(&self) -> &str {
        self.owners.first().map(|o| o.uid.as_str()).unwrap_or(&self.owner_id)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateNftRequest {
    pub title: String,
    pub description: String,
    pub batch_name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub category: Option<String>,
    pub blockchain: String,
    pub royalty: f64,
    pub price: Option<f64>,
    pub currency: String,
    pub for_sale: bool,
    /// Number of on-chain editions to create (None or 1 = regular 1-of-1 mint).
    pub edition_count: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BatchItemMeta {
    pub title: Option<String>,
    pub description: Option<String>,
    pub price: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchNftInput {
    pub batch_name: Option<String>,
    pub blockchain: String,
    pub currency: String,
    pub royalty: f64,
    pub for_sale: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub items: Vec<BatchItemMeta>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateNftRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub category: Option<String>,
    pub price: Option<f64>,
    pub for_sale: Option<bool>,
    pub currency: Option<String>,
    pub mint_address: Option<String>,
}

/// Body for `POST /api/nfts/:id/transfer`.
/// Called by the buyer after an on-chain Solana transaction confirms.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferNftRequest {
    /// Firebase UID of the current owner (seller).
    pub seller_id: String,
    /// Firestore document ID of the marketplace post to mark as sold.
    pub post_id: String,
    /// Confirmed Solana transaction signature for this purchase.
    pub signature: String,
    /// Connected Phantom address that signed the transaction.
    pub payer_address: String,
    /// Short-lived server quote that fixes the exact SOL amount.
    pub quote_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyNftRequest {
    pub nft_id: String,
    pub owner_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyNftResponse {
    pub issued_by_idenity: bool,
    pub issuer_verified: bool,
    pub issuer_name: String,
    pub nft: Nft,
}

/// Body for `POST /api/nfts/:id/payment-quote`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentQuoteRequest {
    /// Firestore marketplace post that contains the authoritative price.
    pub post_id: String,
}

/// Response from `POST /api/nfts/editions`.
/// The frontend uses this to drive the on-chain Master Edition + printV1 calls.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateEditionResponse {
    /// Firestore ID of the master NFT record. ICP has no master editions, so it is absent there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_id:     Option<String>,
    /// Shared off-chain metadata URI (used for every on-chain mint).
    pub metadata_uri:  String,
    /// Firebase Storage URL of the uploaded image (used to create the feed post).
    pub image_url:     String,
    /// Firestore IDs of the N placeholder print-edition records (in order 1..N).
    pub edition_ids:   Vec<String>,
    /// How many print editions were requested.
    pub edition_count: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchUploadResponse {
    pub created: usize,
    pub failed: usize,
    pub results: Vec<BatchItemResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchItemResult {
    pub index: usize,
    pub id: Option<String>,
    pub status: String,
    pub message: Option<String>,
    /// Firebase Storage URL of the uploaded image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    /// Off-chain Metaplex metadata JSON URL — present on success, absent on error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_uri: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nft(owner: &str) -> Nft {
        serde_json::from_value(serde_json::json!({
            "id": "n1", "title": "Water", "description": "", "image": "",
            "category": null, "blockchain": "icp", "royalty": 0.0,
            "ownerId": owner, "ownerName": "Water", "price": null, "currency": null,
            "createdAt": "2026-10-12T12:18:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn chain_starts_with_issuer_and_grows() {
        let a = nft("water");
        assert_eq!(a.issuer_id(), "water");
        let b = Nft { owners: a.owners_with("alan", "Алан", None, "gift", "t1"), owner_id: "alan".into(), ..a };
        assert_eq!(b.owners.iter().map(|o| o.via.as_str()).collect::<Vec<_>>(), ["issued", "gift"]);
        assert_eq!(b.issuer_id(), "water");
        let c = b.owners_with("bob", "Bob", Some("bob".into()), "sale", "t2");
        assert_eq!(c.iter().map(|o| o.uid.as_str()).collect::<Vec<_>>(), ["water", "alan", "bob"]);
    }
}
