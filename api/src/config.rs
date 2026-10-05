use anyhow::{Context, Result};
use std::sync::Arc;

/// Firebase service account credentials parsed from the JSON key file.
#[derive(Debug, Clone)]
pub struct ServiceAccount {
    pub client_email: String,
    pub private_key: String,
    pub token_uri: String,
}

/// Application configuration loaded from environment variables at startup.
/// Wrap in `Arc<Config>` and share across all services.
#[derive(Debug, Clone)]
pub struct Config {
    pub firebase_project_id: String,
    /// Web API key — used for Firebase Auth REST calls (password change, etc.).
    pub firebase_api_key: String,
    /// e.g. `your-project-id.appspot.com`
    pub firebase_storage_bucket: String,
    pub service_account: ServiceAccount,
    /// e.g. `https://api.devnet.solana.com` or `https://api.mainnet-beta.solana.com`
    pub solana_rpc_url: String,
    pub port: u16,
    /// The React dev/prod origin allowed by CORS, e.g. `http://localhost:3000`.
    pub allowed_origin: String,
    /// Firebase UIDs allowed to access `/api/admin/*`. Comma-separated in env.
    pub admin_uids: Vec<String>,
    /// Public NFT viewer URL encoded into QR codes, e.g. `https://example.com/idenity/`.
    pub qr_base_url: String,
}

impl Config {
    pub fn is_admin(&self, uid: &str) -> bool {
        self.admin_uids.iter().any(|a| a == uid)
    }

    /// Build Config from environment variables.
    /// Panics at startup (via `?`) if any required variable is missing or malformed.
    pub fn from_env() -> Result<Arc<Self>> {
        // Parse service account JSON ─────────────────────────────────────────
        let sa_json = std::env::var("FIREBASE_SERVICE_ACCOUNT_JSON")
            .context("FIREBASE_SERVICE_ACCOUNT_JSON must be set")?;

        let sa: serde_json::Value =
            serde_json::from_str(&sa_json).context("FIREBASE_SERVICE_ACCOUNT_JSON is not valid JSON")?;

        let get_str = |key: &str| -> Result<String> {
            sa[key]
                .as_str()
                .map(str::to_owned)
                .with_context(|| format!("Missing '{}' in service account JSON", key))
        };

        let service_account = ServiceAccount {
            client_email: get_str("client_email")?,
            // The JSON key file stores newlines as literal `\n` sequences; serde
            // has already un-escaped them to real newlines by this point, so the
            // PEM is valid for jsonwebtoken.
            private_key: get_str("private_key")?,
            token_uri: sa["token_uri"]
                .as_str()
                .unwrap_or("https://oauth2.googleapis.com/token")
                .to_owned(),
        };

        Ok(Arc::new(Config {
            firebase_project_id: std::env::var("FIREBASE_PROJECT_ID")
                .context("FIREBASE_PROJECT_ID must be set")?,
            firebase_api_key: std::env::var("FIREBASE_API_KEY")
                .context("FIREBASE_API_KEY must be set")?,
            firebase_storage_bucket: std::env::var("FIREBASE_STORAGE_BUCKET")
                .context("FIREBASE_STORAGE_BUCKET must be set")?,
            service_account,
            solana_rpc_url: std::env::var("SOLANA_RPC_URL")
                .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_owned()),
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "8080".to_owned())
                .parse::<u16>()
                .context("PORT must be a valid u16")?,
            allowed_origin: std::env::var("ALLOWED_ORIGIN")
                .unwrap_or_else(|_| "http://localhost:3000".to_owned()),
            admin_uids: std::env::var("ADMIN_UIDS")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect(),
            qr_base_url: std::env::var("QR_BASE_URL")
                .unwrap_or_else(|_| "https://alankharisov.github.io/idenity/".to_owned()),
        }))
    }
}

