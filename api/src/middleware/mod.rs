pub mod auth;

// Re-exported for convenience — used via `crate::middleware::auth::*` throughout.
#[allow(unused_imports)]
pub use auth::{auth_middleware, AuthenticatedUser, JwksVerifier};
