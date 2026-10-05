use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, NaiveDate, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::{
    errors::{ApiResult, AppError},
    models::{Nft, VerifyNftResponse},
    AppState,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicProfile {
    uid: String,
    name: String,
    username: String,
    avatar: Option<String>,
    bio: Option<String>,
    location: Option<String>,
    company_approved: bool,
    followers_count: u64,
    following_count: u64,
    likes_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicNft {
    id: String,
    title: String,
    image: String,
    batch_name: Option<String>,
    created_at: String,
    views: u64,
    has_nfc: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalyticsDay {
    date: String,
    profile_views: u64,
    passport_views: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TopPassport {
    id: String,
    title: String,
    image: String,
    views: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserAnalytics {
    profile_views: u64,
    passport_views: u64,
    followers_count: u64,
    likes_count: u64,
    days: Vec<AnalyticsDay>,
    top_passports: Vec<TopPassport>,
}

fn text(doc: &Value, key: &str) -> String {
    doc.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn optional_text(doc: &Value, key: &str) -> Option<String> {
    doc.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn count(doc: &Value, key: &str) -> u64 {
    doc.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn parse_nfts(doc: &Value) -> Vec<Nft> {
    doc.get("nfts")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// Passport views live in the wallet's `nftViews` map (nft id -> count), not in
/// the `nfts` array: a public view must never rewrite the owner's NFT list, or a
/// view racing with an owner edit could drop that edit. `views` on older records
/// is kept as a floor.
pub fn nft_views(wallet: &Value, nft: &Nft) -> u64 {
    wallet
        .get("nftViews")
        .and_then(|m| m.get(&nft.id))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .max(nft.views)
}

fn find_nft(nfts: &[Nft], nft_id: &str, owner_id: &str) -> Option<Nft> {
    nfts.iter()
        .find(|n| n.id == nft_id && n.owner_id == owner_id)
        .cloned()
}

/// Produces the ISO date keys for a 30-day chart, oldest first, including today.
fn last_30_days(today: NaiveDate) -> Vec<NaiveDate> {
    (0..30)
        .rev()
        .filter_map(|offset| today.checked_sub_signed(Duration::days(offset)))
        .collect()
}

fn sum_buckets(map: Option<&Value>, dates: &[NaiveDate]) -> u64 {
    dates
        .iter()
        .map(|date| {
            map.and_then(|m| m.get(date.format("%Y-%m-%d").to_string()))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
        .sum()
}

fn bucket_value(map: Option<&Value>, date: &str) -> u64 {
    map.and_then(|m| m.get(date))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

pub async fn get_profile(
    State(state): State<Arc<AppState>>,
    Path(username): Path<String>,
) -> ApiResult<Json<PublicProfile>> {
    let docs = state
        .firestore
        .query("users", vec![], None, None)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let wanted = username.to_lowercase();
    let doc = docs
        .into_iter()
        .find(|doc| text(doc, "username").to_lowercase() == wanted)
        .ok_or_else(|| AppError::NotFound("User profile not found".into()))?;
    Ok(Json(PublicProfile {
        uid: text(&doc, "uid"),
        name: text(&doc, "name"),
        username: text(&doc, "username"),
        avatar: optional_text(&doc, "avatar"),
        bio: optional_text(&doc, "bio"),
        location: optional_text(&doc, "location"),
        company_approved: doc
            .get("companyApproved")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        followers_count: count(&doc, "followersCount"),
        following_count: count(&doc, "followingCount"),
        likes_count: crate::handlers::users::likes_count_for_user(&state, &text(&doc, "uid")).await?,
    }))
}

pub async fn get_user_nfts(
    State(state): State<Arc<AppState>>,
    Path(username): Path<String>,
) -> ApiResult<Json<Vec<PublicNft>>> {
    let docs = state
        .firestore
        .query("users", vec![], None, None)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let wanted = username.to_lowercase();
    let user = docs
        .into_iter()
        .find(|doc| text(doc, "username").to_lowercase() == wanted)
        .ok_or_else(|| AppError::NotFound("User profile not found".into()))?;
    let uid = text(&user, "uid");
    let wallet = state
        .firestore
        .get("marki_wallets", &uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({}));
    let nfc_bindings = wallet.get("nfcBindings");
    let mut nfts: Vec<PublicNft> = parse_nfts(&wallet)
        .into_iter()
        .map(|nft| PublicNft {
            views: nft_views(&wallet, &nft),
        has_nfc: nfc_bindings
            .and_then(|m| m.get(&nft.id))
            .map(|v| v.as_str().map(|s| !s.is_empty()).unwrap_or(!v.is_null()))
            .unwrap_or(false),
            id: nft.id,
            title: nft.title,
            image: nft.image,
            batch_name: nft.batch_name,
            created_at: nft.created_at,
        })
        .collect();
    nfts.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(Json(nfts))
}

async fn verified_nft(
    state: &AppState,
    owner_id: &str,
    nft_id: &str,
) -> ApiResult<VerifyNftResponse> {
    let wallet = state
        .firestore
        .get("marki_wallets", owner_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({"nfts": []}));
    let nft = find_nft(&parse_nfts(&wallet), nft_id, owner_id)
        .ok_or_else(|| AppError::NotFound("This QR is not registered by Idenity".into()))?;
    let issuer = state
        .firestore
        .get("users", owner_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Issuer profile not found".into()))?;
    Ok(VerifyNftResponse {
        issued_by_idenity: true,
        issuer_verified: issuer
            .get("companyApproved")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        issuer_name: issuer
            .get("companyName")
            .or_else(|| issuer.get("name"))
            .and_then(Value::as_str)
            .unwrap_or(&nft.owner_name)
            .to_owned(),
        nft,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicPassport {
    #[serde(flatten)]
    verification: VerifyNftResponse,
    owner_name: String,
}

pub async fn get_passport(
    State(state): State<Arc<AppState>>,
    Path((owner_id, nft_id)): Path<(String, String)>,
) -> ApiResult<Json<PublicPassport>> {
    let verification = verified_nft(&state, &owner_id, &nft_id).await?;
    let owner_name = state.firestore.get("users", &owner_id).await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .and_then(|u| u.get("name").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or(verification.nft.owner_name.clone());
    Ok(Json(PublicPassport {
        verification,
        owner_name,
    }))
}

/// Increments a document counter and its current UTC-day bucket.
async fn increment_view(
    state: &AppState,
    collection: &str,
    owner_id: &str,
    field: &str,
) -> ApiResult<()> {
    let mut document = state
        .firestore
        .get(collection, owner_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({}));
    let total = count(&document, field).saturating_add(1);
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let daily_field = format!("{}ByDay", field);
    let mut buckets = document
        .get(&daily_field)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let today = buckets
        .get(&date)
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .saturating_add(1);
    buckets.insert(date, json!(today));
    document[field] = json!(total);
    document[&daily_field] = Value::Object(buckets);
    let mut fields = serde_json::Map::new();
    fields.insert(field.to_owned(), json!(total));
    fields.insert(daily_field, document[&format!("{}ByDay", field)].clone());
    state
        .firestore
        .update(collection, owner_id, &Value::Object(fields))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    Ok(())
}

pub async fn view_profile(
    State(state): State<Arc<AppState>>,
    Path(username): Path<String>,
) -> ApiResult<StatusCode> {
    let docs = state
        .firestore
        .query("users", vec![], None, None)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    let wanted = username.to_lowercase();
    let owner_id = docs
        .into_iter()
        .find(|doc| text(doc, "username").to_lowercase() == wanted)
        .map(|doc| text(&doc, "uid"))
        .ok_or_else(|| AppError::NotFound("User profile not found".into()))?;
    increment_view(&state, "users", &owner_id, "profileViews").await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn view_passport(
    State(state): State<Arc<AppState>>,
    Path((owner_id, nft_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let wallet = state
        .firestore
        .get("marki_wallets", &owner_id)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({"nfts": []}));
    let nft = find_nft(&parse_nfts(&wallet), &nft_id, &owner_id)
        .ok_or_else(|| AppError::NotFound("This QR is not registered by Idenity".into()))?;
    // Field-mask update of the `nftViews` map only; the `nfts` array is untouched.
    let mut views = wallet
        .get("nftViews")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    views.insert(nft.id.clone(), json!(nft_views(&wallet, &nft).saturating_add(1)));
    state
        .firestore
        .update("marki_wallets", &owner_id, &json!({"nftViews": views}))
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?;
    increment_view(&state, "marki_wallets", &owner_id, "passportViews").await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn analytics(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth): axum::Extension<crate::middleware::auth::AuthenticatedUser>,
) -> ApiResult<Json<UserAnalytics>> {
    let wallet = state
        .firestore
        .get("marki_wallets", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({}));
    let user = state
        .firestore
        .get("users", &auth.uid)
        .await
        .map_err(|e| AppError::Firebase(e.to_string()))?
        .unwrap_or_else(|| json!({}));
    let today = Utc::now().date_naive();
    let dates = last_30_days(today);
    let profile_map = user.get("profileViewsByDay");
    let passport_map = wallet.get("passportViewsByDay");
    let days = dates
        .iter()
        .map(|date| {
            let key = date.format("%Y-%m-%d").to_string();
            AnalyticsDay {
                date: key.clone(),
                profile_views: bucket_value(profile_map, &key),
                passport_views: bucket_value(passport_map, &key),
            }
        })
        .collect();
    let top_passports = parse_nfts(&wallet)
        .into_iter()
        .map(|n| (nft_views(&wallet, &n), n))
        .filter(|(views, _)| *views > 0)
        .map(|(views, n)| TopPassport {
            id: n.id,
            title: n.title,
            image: n.image,
            views,
        })
        .collect::<Vec<_>>();
    let mut top_passports = top_passports;
    top_passports.sort_by(|a, b| b.views.cmp(&a.views));
    top_passports.truncate(5);
    Ok(Json(UserAnalytics {
        profile_views: count(&user, "profileViews"),
        passport_views: count(&wallet, "passportViews"),
        followers_count: count(&user, "followersCount"),
        likes_count: crate::handlers::users::likes_count_for_user(&state, &auth.uid).await?,
        days,
        top_passports,
    }))
}

#[cfg(test)]
mod tests {
    use super::{last_30_days, sum_buckets, nft_views};
    use crate::models::Nft;
    use chrono::NaiveDate;
    use serde_json::json;

    #[test]
    fn day_buckets_cover_last_30_days_oldest_first_and_sum_missing_as_zero() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let dates = last_30_days(today);
        assert_eq!(dates.len(), 30);
        assert_eq!(dates[0].to_string(), "2026-08-30");
        assert_eq!(dates[29], today);
        let buckets = json!({"2026-08-30": 4, "2026-09-28": 3, "2026-07-01": 100});
        assert_eq!(sum_buckets(Some(&buckets), &dates), 7);
        assert_eq!(sum_buckets(None, &dates), 0);
    }

    #[test]
    fn nft_views_reads_map_and_keeps_legacy_floor() {
        let mut nft: Nft = serde_json::from_value(serde_json::json!({
            "id": "a", "title": "t", "description": "", "image": "", "ownerId": "o",
            "ownerName": "", "createdAt": "", "views": 3
        })).unwrap();
        let wallet = serde_json::json!({ "nftViews": { "a": 10 } });
        assert_eq!(nft_views(&wallet, &nft), 10);
        nft.views = 12;
        assert_eq!(nft_views(&wallet, &nft), 12);
        assert_eq!(nft_views(&serde_json::json!({}), &nft), 12);
    }
}
