use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::Config;

/// Google OAuth2 scopes required by the backend.
const SCOPES: &str = concat!(
    "https://www.googleapis.com/auth/datastore ",
    "https://www.googleapis.com/auth/devstorage.full_control"
);

// ── Internal types ────────────────────────────────────────────────────────────

/// JWT claims for the service-account bearer-grant flow.
#[derive(Serialize)]
struct ServiceAccountClaims {
    iss: String,
    scope: String,
    aud: String,
    iat: i64,
    exp: i64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    // expires_in is ~3600 s; we subtract a 60-second safety margin in the cache.
}

struct CachedToken {
    access_token: String,
    /// Wall-clock expiry with the safety margin already applied.
    expires_at: chrono::DateTime<Utc>,
}

// ── Public service ────────────────────────────────────────────────────────────

/// Manages a single Google OAuth2 access token shared by `FirestoreClient` and
/// `StorageClient`.  The token is fetched lazily on first use and refreshed
/// automatically when it is within 60 s of expiry.
pub struct GoogleAuthService {
    http: reqwest::Client,
    client_email: String,
    private_key_pem: String,
    token_uri: String,
    cache: RwLock<Option<CachedToken>>,
}

impl GoogleAuthService {
    pub fn new(config: Arc<Config>) -> Result<Self> {
        Ok(GoogleAuthService {
            http: reqwest::Client::new(),
            client_email: config.service_account.client_email.clone(),
            private_key_pem: config.service_account.private_key.clone(),
            token_uri: config.service_account.token_uri.clone(),
            cache: RwLock::new(None),
        })
    }

    /// Return a valid access token, refreshing from Google if necessary.
    pub async fn access_token(&self) -> Result<String> {
        // Fast path: check the read lock first.
        {
            let guard = self.cache.read().await;
            if let Some(cached) = guard.as_ref() {
                if Utc::now() < cached.expires_at {
                    return Ok(cached.access_token.clone());
                }
            }
        }

        // Slow path: refresh.
        let token = self.fetch_token().await?;

        // Store in cache under write lock.
        {
            let mut guard = self.cache.write().await;
            *guard = Some(CachedToken {
                access_token: token.clone(),
                // Google tokens live for 3600 s; cache for 3540 s (1 min safety).
                expires_at: Utc::now() + Duration::seconds(3540),
            });
        }

        Ok(token)
    }

    /// Perform the JWT bearer-grant exchange against Google's token endpoint.
    async fn fetch_token(&self) -> Result<String> {
        let now = Utc::now().timestamp();

        let claims = ServiceAccountClaims {
            iss: self.client_email.clone(),
            scope: SCOPES.to_owned(),
            aud: self.token_uri.clone(),
            iat: now,
            exp: now + 3600,
        };

        let header = Header::new(Algorithm::RS256);
        let encoding_key = EncodingKey::from_rsa_pem(self.private_key_pem.as_bytes())
            .context("Failed to parse RSA private key from service account JSON")?;

        let jwt = jsonwebtoken::encode(&header, &claims, &encoding_key)
            .context("Failed to sign service account JWT")?;

        let response = self
            .http
            .post(&self.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", jwt.as_str()),
            ])
            .send()
            .await
            .context("HTTP request to Google token endpoint failed")?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Google token endpoint returned an error: {}",
                body
            ));
        }

        let parsed: TokenResponse = response
            .json()
            .await
            .context("Failed to parse Google token response")?;

        Ok(parsed.access_token)
    }
}
