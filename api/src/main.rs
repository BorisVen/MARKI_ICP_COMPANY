// Suppress "unused" warnings only for stubs/types not yet consumed by handlers.
// Remove once all phases are complete.
#![allow(dead_code)]

use axum::{http::HeaderName, routing::get, Json, Router};
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod errors;
mod handlers;
mod middleware;
mod models;
mod notification_helpers;
mod routes;
mod services;

use config::Config;
use middleware::auth::JwksVerifier;
use services::AppServices;

/// Shared state injected into every Axum handler via `State<Arc<AppState>>`.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub firestore: Arc<services::FirestoreClient>,
    pub storage: Arc<services::StorageClient>,
    pub solana: Arc<services::SolanaService>,
    /// Caches Google's public JWKS for Firebase JWT verification.
    pub jwks: Arc<JwksVerifier>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "idenity_api=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // ── Config ────────────────────────────────────────────────────────────────
    let config = Config::from_env()?;
    tracing::info!(
        project = %config.firebase_project_id,
        port    = config.port,
        admin_uids = ?config.admin_uids,
        "Config loaded"
    );

    // ── Services ──────────────────────────────────────────────────────────────
    let svc = AppServices::new(config.clone())?;

    let state = Arc::new(AppState {
        jwks: Arc::new(JwksVerifier::new(config.firebase_project_id.clone())),
        config: config.clone(),
        firestore: svc.firestore,
        storage: svc.storage,
        solana: svc.solana,
    });

    // ── CORS ──────────────────────────────────────────────────────────────────
    // `Any` for allow_headers sends `*`, which per the CORS spec does NOT cover
    // the `Authorization` header — browsers warn and will soon block it.
    // Listing headers explicitly fixes both the warning and future breakage.
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods(tower_http::cors::Any)
        .allow_headers([
            HeaderName::from_static("authorization"),
            HeaderName::from_static("content-type"),
            HeaderName::from_static("accept"),
            HeaderName::from_static("x-requested-with"),
        ]);

    // ── Router ────────────────────────────────────────────────────────────────
    let app = Router::new()
        .route("/health", get(health))
        .nest("/api", routes::api_router(state.clone()))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    // ── Serve ─────────────────────────────────────────────────────────────────
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on http://{}", addr);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "idenity-api" }))
}
