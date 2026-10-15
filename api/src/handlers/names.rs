//! Current display names for people in ownership chains and gift claims.
//!
//! Chains store each owner's uid with the name they had at that moment. People
//! rename themselves, so names are looked up again (by uid) whenever a chain is
//! shown. Lookups are cached briefly: one chain often repeats the same people.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::{handlers::gifts::Gift, models::Nft, AppState};

const TTL: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct Person {
    pub name: String,
    pub username: Option<String>,
}

static CACHE: LazyLock<Mutex<HashMap<String, (Instant, Option<Person>)>>> = LazyLock::new(Default::default);

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_owned()
}

/// What a profile is shown as: the company name for brands, else the person's name.
fn person(doc: &Value) -> Person {
    let company = text(doc, "companyName");
    let username = text(doc, "username");
    Person {
        name: if company.is_empty() { text(doc, "name") } else { company },
        username: (!username.is_empty()).then_some(username),
    }
}

/// Current names for `uids` (unknown or missing uids are left out).
pub async fn lookup(state: &AppState, uids: Vec<String>) -> HashMap<String, Person> {
    let mut out = HashMap::new();
    for uid in uids {
        if uid.is_empty() || out.contains_key(&uid) {
            continue;
        }
        let cached = CACHE.lock().unwrap().get(&uid).filter(|(at, _)| at.elapsed() < TTL).map(|(_, p)| p.clone());
        let found = match cached {
            Some(p) => p,
            None => {
                let doc = state.firestore.get("users", &uid).await.ok().flatten();
                let p = doc.as_ref().map(person).filter(|p| !p.name.is_empty());
                let mut cache = CACHE.lock().unwrap();
                if cache.len() > 5_000 {
                    cache.clear();
                }
                cache.insert(uid.clone(), (Instant::now(), p.clone()));
                p
            }
        };
        if let Some(p) = found {
            out.insert(uid, p);
        }
    }
    out
}

/// Replaces stored owner names in NFT chains with the owners' current names.
pub async fn refresh_nfts(state: &AppState, nfts: &mut [Nft]) {
    let uids: Vec<String> = nfts.iter().flat_map(|n| n.owners.iter().map(|o| o.uid.clone())).collect();
    let names = lookup(state, uids).await;
    for o in nfts.iter_mut().flat_map(|n| n.owners.iter_mut()) {
        if let Some(p) = names.get(&o.uid) {
            o.name = p.name.clone();
            o.username = p.username.clone();
        }
    }
}

/// Same for gifts: who claimed, and who sent.
pub async fn refresh_gifts(state: &AppState, gifts: &mut [Gift]) {
    let uids: Vec<String> = gifts.iter().flat_map(|g| g.claims.iter().map(|c| c.uid.clone()).chain([g.owner_id.clone()])).collect();
    let names = lookup(state, uids).await;
    for g in gifts.iter_mut() {
        if let Some(p) = names.get(&g.owner_id) {
            g.owner_name = p.name.clone();
            g.owner_username = p.username.clone().unwrap_or_default();
        }
        for c in g.claims.iter_mut() {
            if let Some(p) = names.get(&c.uid) {
                c.name = p.name.clone();
                c.username = p.username.clone().unwrap_or_default();
            }
        }
    }
}
