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
            "data": {
                "type": kind,
                "id": notification["id"].as_str().unwrap_or_default(),
                "action": notification["metadata"]["action"].as_str().unwrap_or_default(),
                "target": notification["metadata"]["target"].as_str().unwrap_or_default(),
            },
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
        // `PUSH_ACTION=likes|comments|gift-requests`: the push opens that (on the user's newest post).
        let action = std::env::var("PUSH_ACTION").unwrap_or_default();
        let post = db.query("posts", vec![QueryFilter::equal("userId", uid.clone())], None, None).await.unwrap_or_default()
            .into_iter().max_by(|a, b| a["createdAt"].as_str().cmp(&b["createdAt"].as_str()));
        let (target, title) = (post.as_ref().and_then(|p| p["id"].as_str()).unwrap_or_default().to_owned(),
            post.as_ref().and_then(|p| p["title"].as_str()).unwrap_or("NFT").to_owned());
        println!("action={action} post={target} «{title}»");
        let (head, text, kind) = match action.as_str() {
            "comments" => ("💬 HarisAlan · «".to_owned() + &title + "»", "Класна річ! 🔥".to_owned(), "comment"),
            "gift-requests" => ("Запит на подарунок 🎁".to_owned(), format!("HarisAlan просить «{title}»"), "gift"),
            _ => ("❤️ HarisAlan".to_owned(), format!("Подобається ваш NFT «{title}»"), "like"),
        };
        send(&db, &json!({ "id": format!("test-{}", chrono::Utc::now().timestamp_millis()), "userId": uid, "title": head, "text": text, "type": kind,
            "metadata": { "action": if action.is_empty() { "likes" } else { action.as_str() }, "target": target } })).await;
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

#[cfg(test)]
mod migrate_texts {
    use crate::services::{FirestoreClient, GoogleAuthService};
    use serde_json::{json, Value};
    use std::sync::Arc;

    fn s(v: &Value) -> String { v.as_str().unwrap_or_default().to_owned() }
    fn money(m: &Value) -> String {
        match (m["price"].as_f64(), m["currency"].as_str()) { (Some(p), Some(c)) => format!(" за {p} {c}"), _ => String::new() }
    }

    /// Old English notifications → the current Ukrainian texts, and no buyer
    /// phone/address in order texts. Written without push (set_raw).
    /// Dry run: `cargo test migrate_notifications_live -- --ignored --nocapture`; `APPLY=1` writes.
    #[tokio::test]
    #[ignore]
    async fn migrate_notifications_live() {
        dotenvy::dotenv().ok();
        let config = crate::config::Config::from_env().unwrap();
        let db = FirestoreClient::new(config.clone(), Arc::new(GoogleAuthService::new(config).unwrap()));
        let apply = std::env::var("APPLY").is_ok_and(|v| v == "1");
        let all = db.query("notifications", vec![], None, None).await.unwrap();
        let (mut changed, mut total) = (0, 0);
        for mut n in all {
            total += 1;
            let (title, m) = (s(&n["title"]), n["metadata"].clone());
            let (t, from) = (s(&m["nftTitle"]), s(&m["fromUser"]));
            let new: Option<(String, String)> = match title.as_str() {
                "Someone liked your NFT ❤️" => Some((format!("❤️ {from}"), format!("Подобається ваш NFT «{t}»"))),
                "New comment on your NFT 💬" => Some((format!("💬 {from} · «{t}»"), "Новий коментар".into())),
                "Purchase successful 🛒" => Some(("🛍️ Покупка успішна".into(), format!("Ви купили «{t}»{}", money(&m)))),
                "Your NFT was sold 💰" => Some(("💰 Ваш NFT продано".into(), format!("«{t}» купив {from}{}", money(&m)))),
                "Welcome to Idenity 🎉" => Some(("👋 Вітаємо в Marki".into(), "Створюйте паспорти речей, даруйте й продавайте NFT.".into())),
                "NFT created 🎨" => Some(("🎨 NFT створено".into(), format!("«{t}» уже в блокчейні"))),
                "Order placed 📦" => Some(("📦 Замовлення оформлено".into(), format!("«{t}»{} — продавець скоро відправить", money(&m)))),
                "New COD order 🚚" => Some(("🚚 Нове замовлення".into(), format!("{from} замовив «{t}»{}. Деталі доставки — у «Замовленнях»", money(&m)))),
                "You received a gift 🎁" => Some(("🎁 Вам подарунок".into(), format!("«{t}» уже у ваших паспортах"))),
                "Your gift was claimed 🎁" => Some(("🎁 Подарунок забрали".into(), format!("«{t}» забрали"))),
                x if x.ends_with("NFTs created 🎨") => Some((format!("🎨 Створено {} NFT", x.split_whitespace().next().unwrap_or("")), "Партія вже в блокчейні".into())),
                _ => None,
            };
            let Some((nt, nx)) = new else { continue };
            changed += 1;
            if changed <= 6 { println!("{title}  →  {nt} | {nx}"); }
            if apply {
                n["title"] = json!(nt);
                n["text"] = json!(nx);
                let id = s(&n["id"]);
                db.set_raw("notifications", &id, &n).await.unwrap();
            }
        }
        println!("total {total}, to change {changed}, applied: {apply}");
    }
}
