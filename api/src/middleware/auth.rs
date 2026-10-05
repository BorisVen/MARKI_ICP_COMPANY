use anyhow::{Context, Result};
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
    Json,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::AppState;

// ── JWKS cache ────────────────────────────────────────────────────────────────

const GOOGLE_JWKS_URI: &str =
    "https://www.googleapis.com/robot/v1/metadata/jwk/securetoken@system.gserviceaccount.com";

struct CachedJwks {
    key_set: JwkSet,
    expires_at: chrono::DateTime<Utc>,
}

/// Fetches and caches Google's public JWKS used to verify Firebase ID tokens.
/// Keys are cached for 1 hour; Google rotates them every few days.
pub struct JwksVerifier {
    http: reqwest::Client,
    project_id: String,
    cache: RwLock<Option<CachedJwks>>,
}

#[derive(Deserialize)]
struct FirebaseClaims {
    /// Firebase UID — the `sub` claim equals the user's UID.
    sub: String,
}

impl JwksVerifier {
    pub fn new(project_id: String) -> Self {
        JwksVerifier {
            http: reqwest::Client::new(),
            project_id,
            cache: RwLock::new(None),
        }
    }

    /// Verify a Firebase ID token and return the UID on success.
    pub async fn verify(&self, token: &str) -> Result<String> {
        let jwks = self.get_jwks().await?;

        let header = decode_header(token).context("Failed to decode JWT header")?;

        let kid = header.kid.ok_or_else(|| anyhow::anyhow!("JWT has no 'kid' header"))?;

        let jwk = jwks
            .find(&kid)
            .ok_or_else(|| anyhow::anyhow!("No JWK found matching kid '{}'", kid))?;

        let decoding_key =
            DecodingKey::from_jwk(jwk).context("Failed to build DecodingKey from JWK")?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.project_id]);
        validation.set_issuer(&[format!(
            "https://securetoken.google.com/{}",
            self.project_id
        )]);

        let token_data = decode::<FirebaseClaims>(token, &decoding_key, &validation)
            .context("JWT signature/claims validation failed")?;

        Ok(token_data.claims.sub)
    }

    async fn get_jwks(&self) -> Result<JwkSet> {
        // Fast path — read lock only.
        {
            let guard = self.cache.read().await;
            if let Some(cached) = guard.as_ref() {
                if Utc::now() < cached.expires_at {
                    return Ok(cached.key_set.clone());
                }
            }
        }

        // Refresh.
        let resp = self
            .http
            .get(GOOGLE_JWKS_URI)
            .send()
            .await
            .context("HTTP request for Google JWKS failed")?;

        if !resp.status().is_success() {
            return Err(anyhow::anyhow!(
                "JWKS endpoint returned {}",
                resp.status()
            ));
        }

        let jwks: JwkSet = resp.json().await.context("Failed to parse JWKS JSON")?;

        {
            let mut guard = self.cache.write().await;
            *guard = Some(CachedJwks {
                key_set: jwks.clone(),
                // Google docs say keys are valid for ~1 week; refresh hourly.
                expires_at: Utc::now() + Duration::hours(1),
            });
        }

        Ok(jwks)
    }
}

// ── Extractor ─────────────────────────────────────────────────────────────────

/// Injected into request extensions by `auth_middleware`.
/// Available in handlers via `Extension<AuthenticatedUser>`.
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub uid: String,
    /// Raw Bearer token — needed for Firebase Auth REST calls
    /// (password change, email update) that require the user's own token.
    pub token: String,
}

// ── Middleware function ───────────────────────────────────────────────────────

type AuthError = (StatusCode, Json<serde_json::Value>);

fn unauth(msg: &str) -> AuthError {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": msg })),
    )
}

pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Result<Response, AuthError> {
    let auth_value = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| unauth("Missing Authorization header"))?;

    let token = auth_value
        .strip_prefix("Bearer ")
        .ok_or_else(|| unauth("Authorization header must use Bearer scheme"))?
        .to_owned();

    let uid = state
        .jwks
        .verify(&token)
        .await
        .map_err(|e| {
            tracing::debug!(error = %e, "JWT verification failed");
            unauth("Invalid or expired token")
        })?;

    req.extensions_mut().insert(AuthenticatedUser { uid, token });
    Ok(next.run(req).await)
}
