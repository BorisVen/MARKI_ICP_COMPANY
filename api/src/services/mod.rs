pub mod firestore;
pub mod google_auth;
pub mod solana;
pub mod storage;

pub use firestore::FirestoreClient;
pub use google_auth::GoogleAuthService;
pub use solana::SolanaService;
pub use storage::StorageClient;

use anyhow::Result;
use std::sync::Arc;

use crate::config::Config;

/// All backend service clients bundled together for easy construction and
/// injection into `AppState`.
pub struct AppServices {
    pub firestore: Arc<FirestoreClient>,
    pub storage: Arc<StorageClient>,
    pub solana: Arc<SolanaService>,
}

impl AppServices {
    /// Initialise all services from the shared config.
    /// The `GoogleAuthService` (OAuth2 token cache) is constructed once and
    /// shared by both `FirestoreClient` and `StorageClient`.
    pub fn new(config: Arc<Config>) -> Result<Self> {
        let auth = Arc::new(GoogleAuthService::new(config.clone())?);

        Ok(AppServices {
            firestore: Arc::new(FirestoreClient::new(config.clone(), auth.clone())),
            storage: Arc::new(StorageClient::new(config.clone(), auth)),
            solana: Arc::new(SolanaService::new(config.solana_rpc_url.clone())),
        })
    }
}
