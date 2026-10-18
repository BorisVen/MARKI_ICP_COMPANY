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

#[cfg(test)]
mod find_users {
    use crate::services::{FirestoreClient, GoogleAuthService};
    use std::sync::Arc;

    /// Live: users whose email or name contains one of `FIND` (comma-separated).
    #[tokio::test]
    #[ignore]
    async fn find_users_live() {
        dotenvy::dotenv().ok();
        let config = crate::config::Config::from_env().unwrap();
        let db = FirestoreClient::new(config.clone(), Arc::new(GoogleAuthService::new(config).unwrap()));
        let needles: Vec<String> = std::env::var("FIND").unwrap().split(',').map(|s| s.trim().to_lowercase()).collect();
        for u in db.query("users", vec![], None, Some(5000)).await.unwrap() {
            let (email, name, nick) = (u["email"].as_str().unwrap_or("").to_lowercase(), u["name"].as_str().unwrap_or("").to_lowercase(), u["username"].as_str().unwrap_or(""));
            if needles.iter().any(|n| email.contains(n) || name.contains(n)) {
                println!("{} | {} | @{} | {}", u["uid"].as_str().unwrap_or(""), email, nick, u["name"].as_str().unwrap_or(""));
            }
        }
    }
}

#[cfg(test)]
mod cleanup_test_nfts {
    use crate::services::{FirestoreClient, GoogleAuthService};
    use serde_json::{json, Value};
    use std::collections::{BTreeMap, HashSet};
    use std::sync::Arc;

    /// Minted for real: an ICP token (`icp:<canister>:<id>`) or a Solana mint address.
    /// Mints that exist on a real network: the mainnet ICP canister and the three
    /// Solana mints found on mainnet-beta (the rest were made on devnet).
    const SOLANA_MAINNET: &[&str] = &[
        "5uEgQTmoKyvq2fM9Bn2qabTeqZA2MFufRVCU6BFnCqUk",
        "ELSGC6qmJg2CCZVYidQx2yE5T8g4VhHEXNUVuCVzCwvB",
        "FwxtySFhWsK5KR6MRKvzfPvuqMhHfqj6bkkxJtsWjpc9",
    ];
    pub fn real_mint(m: &str) -> bool {
        let m = m.trim();
        m.starts_with("icp:dbrm3-yqaaa-aaaak-qzjeq-cai:") || SOLANA_MAINNET.contains(&m)
    }

    fn nfts(doc: &Value) -> Vec<Value> { doc["nfts"].as_array().cloned().unwrap_or_default() }

    /// Test NFTs (never minted) and the posts made only of them.
    /// Dry run: `cargo test cleanup_test_nfts_live -- --ignored --nocapture`.
    /// `APPLY=1` first saves everything it removes to `BACKUP` (a JSON file), then removes.
    #[tokio::test]
    #[ignore]
    async fn cleanup_test_nfts_live() {
        dotenvy::dotenv().ok();
        let config = crate::config::Config::from_env().unwrap();
        let db = FirestoreClient::new(config.clone(), Arc::new(GoogleAuthService::new(config).unwrap()));
        let apply = std::env::var("APPLY").is_ok_and(|v| v == "1");
        let names: BTreeMap<String, String> = db.query("users", vec![], None, Some(5000)).await.unwrap().into_iter()
            .map(|u| (u["uid"].as_str().unwrap_or("").to_owned(), u["username"].as_str().unwrap_or("").to_owned())).collect();

        let mut test_ids = HashSet::new();
        let mut real_ids = HashSet::new();
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        let mut backup = json!({ "wallets": {}, "posts": [] });
        let mut plans: Vec<(String, Vec<Value>, u64)> = vec![];
        for w in db.query("marki_wallets", vec![], None, None).await.unwrap() {
            let uid = w["userId"].as_str().or(w["uid"].as_str()).or(w["_id"].as_str()).or(w["id"].as_str()).unwrap_or("").to_owned();
            if uid.is_empty() { println!("wallet without uid, keys: {:?}", w.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())); continue; }
            let chunks = w["nftChunks"].as_u64().unwrap_or(0);
            let mut all = nfts(&w);
            for k in 1..=chunks {
                if let Some(c) = db.get("marki_wallet_nfts", &format!("{uid}_{k}")).await.unwrap() { all.extend(nfts(&c)); }
            }
            let (keep, drop): (Vec<Value>, Vec<Value>) = all.into_iter().partition(|n| n["mintAddress"].as_str().is_some_and(real_mint));
            for n in &keep { real_ids.insert(n["id"].as_str().unwrap_or("").to_owned()); }
            for n in &drop {
                test_ids.insert(n["id"].as_str().unwrap_or("").to_owned());
                let kind = match n["mintAddress"].as_str() { None => "no mint".to_owned(), Some(m) if m.is_empty() => "empty".into(), Some(m) => m.chars().take(10).collect() };
                *kinds.entry(kind).or_default() += 1;
            }
            if !drop.is_empty() {
                println!("@{:<16} keep {:>5} (icp/sol)  remove {:>5}", names.get(&uid).cloned().unwrap_or(uid.clone()), keep.len(), drop.len());
                backup["wallets"][&uid] = json!(drop);
                plans.push((uid, keep, chunks));
            }
        }
        println!("KINDS of unminted: {kinds:?}");

        let posts = db.query("posts", vec![], None, None).await.unwrap();
        let mut drop_posts = vec![];
        for p in &posts {
            let refs: Vec<String> = p["walletNftIds"].as_array().cloned().unwrap_or_default().iter()
                .chain(std::iter::once(&p["walletNftId"])).filter_map(|v| v.as_str().map(str::to_owned)).filter(|s| !s.is_empty()).collect();
            // Old posts without NFTs are test posts too; a post stays if anything in it is real.
            let only_test = !refs.iter().any(|r| real_ids.contains(r));
            if only_test { drop_posts.push(p.clone()); }
        }
        println!("posts: {} total, remove {} (made only of test NFTs)", posts.len(), drop_posts.len());
        for p in drop_posts.iter().take(12) { println!("   - «{}» by @{}", p["title"].as_str().unwrap_or(""), names.get(p["userId"].as_str().unwrap_or("")).cloned().unwrap_or_default()); }

        if !apply { println!("dry run — nothing changed"); return; }
        backup["posts"] = json!(drop_posts);
        let path = std::env::var("BACKUP").expect("BACKUP=<file> is required to apply");
        std::fs::write(&path, serde_json::to_string_pretty(&backup).unwrap()).unwrap();
        println!("backup written to {path}");
        for p in &drop_posts { db.delete("posts", p["id"].as_str().unwrap()).await.unwrap(); }
        for (uid, keep, old_chunks) in plans {
            let mut parts = keep.chunks(400);
            let first = parts.next().unwrap_or(&[]).to_vec();
            let rest: Vec<Vec<Value>> = parts.map(|c| c.to_vec()).collect();
            for (i, part) in rest.iter().enumerate() {
                db.set_raw("marki_wallet_nfts", &format!("{uid}_{}", i + 1), &json!({ "userId": uid, "nfts": part })).await.unwrap();
            }
            db.update("marki_wallets", &uid, &json!({ "nfts": first, "nftChunks": rest.len() })).await.unwrap();
            for k in rest.len() as u64 + 1..=old_chunks { let _ = db.delete("marki_wallet_nfts", &format!("{uid}_{k}")).await; }
        }
        println!("applied");
    }
}

#[cfg(test)]
mod list_posts {
    use crate::services::{FirestoreClient, GoogleAuthService};
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Live: every post with what it points to (and whether that NFT is minted).
    #[tokio::test]
    #[ignore]
    async fn list_posts_live() {
        dotenvy::dotenv().ok();
        let config = crate::config::Config::from_env().unwrap();
        let db = FirestoreClient::new(config.clone(), Arc::new(GoogleAuthService::new(config).unwrap()));
        let names: HashMap<String, String> = db.query("users", vec![], None, Some(5000)).await.unwrap().into_iter()
            .map(|u| (u["uid"].as_str().unwrap_or("").to_owned(), u["username"].as_str().unwrap_or("").to_owned())).collect();
        let mut mints: HashMap<String, String> = HashMap::new();
        for w in db.query("marki_wallets", vec![], None, None).await.unwrap() {
            let uid = w["userId"].as_str().or(w["id"].as_str()).unwrap_or("").to_owned();
            let mut all = w["nfts"].as_array().cloned().unwrap_or_default();
            for k in 1..=w["nftChunks"].as_u64().unwrap_or(0) {
                if let Some(c) = db.get("marki_wallet_nfts", &format!("{uid}_{k}")).await.unwrap() { all.extend(c["nfts"].as_array().cloned().unwrap_or_default()); }
            }
            for n in all { if std::env::var("FULL").is_ok() { if let Some(m) = n["mintAddress"].as_str() { println!("MINT {m}"); } } mints.insert(n["id"].as_str().unwrap_or("").to_owned(), n["mintAddress"].as_str().unwrap_or("-").chars().take(14).collect()); }
        }
        let mut posts = db.query("posts", vec![], None, None).await.unwrap();
        posts.sort_by(|a, b| b["createdAt"].as_str().cmp(&a["createdAt"].as_str()));
        for p in posts {
            let refs: Vec<String> = p["walletNftIds"].as_array().cloned().unwrap_or_default().iter().chain(std::iter::once(&p["walletNftId"]))
                .filter_map(|v| v.as_str().map(str::to_owned)).filter(|s| !s.is_empty()).collect();
            let state: Vec<String> = refs.iter().take(2).map(|r| mints.get(r).cloned().unwrap_or("GONE".into())).collect();
            println!("{} | @{:<14} | {:<26} | refs {:>3} {:?} | img {}", &p["createdAt"].as_str().unwrap_or("")[..10], names.get(p["userId"].as_str().unwrap_or("")).cloned().unwrap_or_default(),
                p["title"].as_str().unwrap_or("").chars().take(26).collect::<String>(), refs.len(), state, p["nftImage"].as_str().map(|s| !s.is_empty()).unwrap_or(false));
        }
    }
}

#[cfg(test)]
mod remove_orphan_posts {
    use crate::services::{FirestoreClient, GoogleAuthService};
    use std::sync::Arc;

    /// Live: removes posts by their real document name when the title matches `TITLES`.
    #[tokio::test]
    #[ignore]
    async fn remove_posts_by_title_live() {
        dotenvy::dotenv().ok();
        let config = crate::config::Config::from_env().unwrap();
        let db = FirestoreClient::new(config.clone(), Arc::new(GoogleAuthService::new(config).unwrap()));
        let titles: Vec<String> = std::env::var("TITLES").unwrap().split('|').map(str::to_owned).collect();
        for (name, title) in db.names_and_titles("posts").await.unwrap() {
            if titles.contains(&title) {
                println!("delete posts/{name} «{title}»");
                if std::env::var("APPLY").is_ok_and(|v| v == "1") { db.delete("posts", &name).await.unwrap(); }
            }
        }
    }
}

#[cfg(test)]
mod discover_preview {
    use crate::services::{FirestoreClient, GoogleAuthService};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    /// Live: the collections /api/discover would show (owner, title, count).
    #[tokio::test]
    #[ignore]
    async fn discover_preview_live() {
        dotenvy::dotenv().ok();
        let config = crate::config::Config::from_env().unwrap();
        let db = FirestoreClient::new(config.clone(), Arc::new(GoogleAuthService::new(config).unwrap()));
        let names: BTreeMap<String, String> = db.query("users", vec![], None, Some(5000)).await.unwrap().into_iter()
            .map(|u| (u["uid"].as_str().unwrap_or("").to_owned(), u["username"].as_str().unwrap_or("").to_owned())).collect();
        let mut n = 0;
        for w in db.query("marki_wallets", vec![], None, None).await.unwrap() {
            let uid = w["userId"].as_str().or(w["id"].as_str()).unwrap_or("").to_owned();
            let mut all = w["nfts"].as_array().cloned().unwrap_or_default();
            for k in 1..=w["nftChunks"].as_u64().unwrap_or(0) {
                if let Some(c) = db.get("marki_wallet_nfts", &format!("{uid}_{k}")).await.unwrap() { all.extend(c["nfts"].as_array().cloned().unwrap_or_default()); }
            }
            let mut groups: BTreeMap<String, usize> = BTreeMap::new();
            for x in all.iter().filter(|x| x["mintAddress"].as_str().is_some_and(|m| !m.is_empty()) && x["image"].as_str().is_some_and(|i| !i.is_empty())) {
                let t = x["batchName"].as_str().filter(|b| !b.is_empty()).or(x["title"].as_str()).unwrap_or("").to_owned();
                *groups.entry(format!("{t}|{}", x["image"].as_str().unwrap_or(""))).or_default() += 1;
            }
            for (k, c) in groups { n += 1; println!("@{:<14} {:<30} ×{c}", names.get(&uid).cloned().unwrap_or_default(), k.split('|').next().unwrap_or("")); }
        }
        println!("cards: {n}");
    }
}
