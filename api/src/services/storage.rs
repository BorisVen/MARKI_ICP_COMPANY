use anyhow::{Context, Result};
use std::sync::Arc;

use crate::config::Config;
use super::google_auth::GoogleAuthService;

/// Wraps the Firebase Storage REST API for file upload and deletion.
///
/// All operations authenticate using the shared `GoogleAuthService` access
/// token (service-account bearer grant), which gives admin-level access to the
/// bucket — consistent with the backend owning all storage decisions.
pub struct StorageClient {
    http: reqwest::Client,
    bucket: String,
    auth: Arc<GoogleAuthService>,
}

impl StorageClient {
    pub fn new(config: Arc<Config>, auth: Arc<GoogleAuthService>) -> Self {
        StorageClient {
            http: reqwest::Client::new(),
            bucket: config.firebase_storage_bucket.clone(),
            auth,
        }
    }

    /// Upload raw file bytes to Firebase Storage.
    ///
    /// `path`         — object path inside the bucket, e.g. `nfts/{uid}/{id}.jpg`
    /// `data`         — raw file bytes
    /// `content_type` — MIME type, e.g. `image/jpeg`
    ///
    /// Returns the public download URL for the uploaded file.
    pub async fn upload(
        &self,
        path: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<String> {
        let token = self.auth.access_token().await?;

        // The `name` query parameter must be URL-encoded so that `/` in the path
        // becomes `%2F`.
        let encoded_name = urlencoding::encode(path);
        let url = format!(
            "https://firebasestorage.googleapis.com/v0/b/{}/o?name={}",
            self.bucket, encoded_name
        );

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(data)
            .send()
            .await
            .context("Firebase Storage upload request failed")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Storage upload error for '{}': {}", path, body));
        }

        Ok(self.public_url(path))
    }

    /// Delete a file from Firebase Storage.
    /// Succeeds silently if the file does not exist (404 is ignored).
    pub async fn delete(&self, path: &str) -> Result<()> {
        let token = self.auth.access_token().await?;

        // In the DELETE URL the path is part of the URL path and must be
        // percent-encoded (/ → %2F).
        let encoded_name = urlencoding::encode(path);
        let url = format!(
            "https://firebasestorage.googleapis.com/v0/b/{}/o/{}",
            self.bucket, encoded_name
        );

        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .context("Firebase Storage delete request failed")?;

        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Storage delete error for '{}': {}", path, body));
        }

        Ok(())
    }

    /// Build the public `?alt=media` download URL for a storage path.
    ///
    /// Note: the object name in the URL path component is percent-encoded so
    /// that forward-slashes become `%2F`.
    pub fn public_url(&self, path: &str) -> String {
        let encoded = urlencoding::encode(path);
        format!(
            "https://firebasestorage.googleapis.com/v0/b/{}/o/{}?alt=media",
            self.bucket, encoded
        )
    }

    /// Derive a deterministic storage path for an NFT image.
    ///
    /// `owner_id` — Firebase UID of the NFT owner
    /// `nft_id`   — UUID of the NFT
    /// `ext`      — file extension without dot, e.g. `"jpg"`
    pub fn nft_path(owner_id: &str, nft_id: &str, ext: &str) -> String {
        format!("nfts/{}/{}.{}", owner_id, nft_id, ext)
    }

    /// Derive a deterministic storage path for an NFT's off-chain metadata JSON.
    pub fn metadata_path(owner_id: &str, nft_id: &str) -> String {
        format!("nfts/{}/{}_metadata.json", owner_id, nft_id)
    }

    /// Derive a deterministic storage path for a user avatar.
    pub fn avatar_path(user_id: &str, ext: &str) -> String {
        format!("avatars/{}/avatar.{}", user_id, ext)
    }
}
