//! Push notifications to the phone (Firebase Cloud Messaging, HTTP v1).
//!
//! Every in-app notification (like, comment, gift, request, …) is also pushed to
//! the owner's devices. The app registers its FCM token via `PUT /api/me/push-token`;
//! tokens live in `push_tokens/{uid}` and dead ones are dropped on the first failure.

use serde_json::{json, Value};

use super::FirestoreClient;

pub const TOKENS: &str = "push_tokens";

/// Device tokens of `uid`.
pub async fn tokens_of(db: &FirestoreClient, uid: &str) -> Vec<String> {
    db.get(TOKENS, uid).await.ok().flatten()
        .and_then(|d| d["tokens"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|t| t.as_str().map(str::to_owned))
        .collect()
}

/// Pushes a stored notification document (`userId`, `title`, `text`) to its owner.
pub async fn send(db: &FirestoreClient, notification: &Value) {
    let Some(uid) = notification["userId"].as_str() else { return };
    let tokens = tokens_of(db, uid).await;
    if tokens.is_empty() { return; }
    let title = notification["title"].as_str().unwrap_or("Marki");
    let body = notification["text"].as_str().unwrap_or_default();
    let kind = notification["type"].as_str().unwrap_or_default();

    let mut dead = Vec::new();
    for token in &tokens {
        let message = json!({ "message": {
            "token": token,
            "notification": { "title": title, "body": body },
            "data": { "type": kind, "page": "notifications", "id": notification["id"].as_str().unwrap_or_default() },
            "android": { "priority": "high", "notification": { "channel_id": "marki" } },
        }});
        match db.fcm_send(&message).await {
            Ok(()) => {}
            Err(e) if e.to_string().contains("UNREGISTERED") || e.to_string().contains("INVALID_ARGUMENT") => dead.push(token.clone()),
            Err(e) => tracing::warn!(error = %e, "push failed"),
        }
    }
    if !dead.is_empty() {
        let alive: Vec<_> = tokens.into_iter().filter(|t| !dead.contains(t)).collect();
        let _ = db.set_raw(TOKENS, uid, &json!({ "tokens": alive })).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{firestore::QueryFilter, GoogleAuthService};
    use std::sync::Arc;

    /// Live: `PUSH_TEST_USERNAME=alan cargo test push_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn push_live() {
        dotenvy::dotenv().ok();
        let config = crate::config::Config::from_env().unwrap();
        let db = FirestoreClient::new(config.clone(), Arc::new(GoogleAuthService::new(config).unwrap()));
        let username = std::env::var("PUSH_TEST_USERNAME").unwrap();
        let users = db.query("users", vec![QueryFilter::equal("username", username)], None, None).await.unwrap();
        let uid = users[0]["uid"].as_str().unwrap().to_owned();
        println!("tokens: {}", tokens_of(&db, &uid).await.len());
        send(&db, &json!({ "id": format!("test-{}", chrono::Utc::now().timestamp_millis()), "userId": uid, "title": "❤️ water", "text": "Подобається ваш NFT «Marki Genesis #1»", "type": "like" })).await;
        println!("tokens after: {}", tokens_of(&db, &uid).await.len());
    }
}

#[cfg(test)]
mod wallet_check {
    use crate::services::{firestore::QueryFilter, FirestoreClient, GoogleAuthService};
    use std::sync::Arc;

    /// Live: users who linked a wallet. `cargo test linked_wallets_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn linked_wallets_live() {
        dotenvy::dotenv().ok();
        let config = crate::config::Config::from_env().unwrap();
        let db = FirestoreClient::new(config.clone(), Arc::new(GoogleAuthService::new(config).unwrap()));
        for kind in ["ii", "plug"] {
            for u in db.query("users", vec![QueryFilter::equal("icpWalletKind", kind)], None, None).await.unwrap() {
                println!("{kind} @{} uid={} principal={}", u["username"], u["uid"], u["icpPrincipal"]);
            }
        }
        for name in ["water", "alan"] {
            let u = db.query("users", vec![QueryFilter::equal("username", name)], None, None).await.unwrap();
            if let Some(u) = u.first() { println!("@{name} uid={} principal={} kind={}", u["uid"], u["icpPrincipal"], u["icpWalletKind"]); }
        }
    }
}
