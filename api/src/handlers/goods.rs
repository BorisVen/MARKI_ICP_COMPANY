//! Authenticity for physical goods (wine, spirits, dairy, …), separate from NFTs.
//!
//! A brand creates a **product** (model, e.g. "Cabernet 2021"), then **batches**
//! of it. A batch either has one public code for the whole lot (`mode = "batch"`,
//! e.g. milk) or one code per item (`mode = "unit"`, e.g. a wine bottle). The
//! brand records supply-chain **events** on the batch (bottled → warehouse →
//! shipped → in store). Anyone can scan a code and see the product, its history
//! and warnings; a buyer claims a unit by scanning the QR of their fiscal receipt.
//!
//! Firestore: `goods_products/{id}`, `goods_batches/{id}` (units and events are
//! embedded), `goods_receipts/{fn:id}` (units claimed per receipt) and
//! `goods_scans/{batchId}` (scan counters). A unit code is the batch id followed
//! by 8 random chars, so a scan loads exactly one batch document. Batch writes go
//! through one process-wide lock, like gift claims (the API runs as one instance).
//!
//! Clone detection: a printed QR can be copied, so every scan of a unit is logged
//! with a coarse region (the phone's time zone → country) and an anonymous device
//! hash. If people other than the owner scan one code from two countries within a
//! day, the unit is flagged as a probable fake and everyone sees it. The owner and
//! anyone it was handed to (gift / transfer) are trusted and never trigger it.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use base64::Engine;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    services::firestore::QueryFilter,
    AppState,
};

const PRODUCTS: &str = "goods_products";
const BATCHES: &str = "goods_batches";
const RECEIPTS: &str = "goods_receipts";
const SCANS: &str = "goods_scans";

/// Keeps a batch document well under Firestore's 1 MiB limit.
const MAX_UNITS: u32 = 2_000;
/// One receipt may cover several bottles, but not a whole shelf.
const MAX_UNITS_PER_RECEIPT: usize = 24;
const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
/// Scans kept per unit for clone detection.
const MAX_SCANS_PER_UNIT: usize = 40;
/// Two countries within this window means two physical copies of one code.
const CLONE_WINDOW_HOURS: i64 = 24;
/// Previous owners stay trusted for a while after handing the item over.
const TRUSTED_AFTER_HANDOVER_DAYS: i64 = 7;

const CATEGORIES: &[&str] = &["wine", "spirits", "beer", "dairy", "food", "cosmetics", "other"];
/// Supply-chain stages in order; `note` and `lab` don't move the stage.
const STAGES: &[&str] = &["produced", "warehouse", "shipped", "in_store"];
const EVENT_KINDS: &[&str] = &["produced", "warehouse", "shipped", "in_store", "lab", "note"];

static BATCH_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

// ── Stored records ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Product {
    pub id: String,
    pub owner_id: String,
    pub brand_name: String,
    pub name: String,
    pub category: String,
    /// "unit": a code per item; "batch": one code per lot.
    pub mode: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEvent {
    pub kind: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub note: String,
    pub actor_name: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub fiscal_number: String,
    pub check_id: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub sum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unit {
    pub serial: u32,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
    /// Ownership chain: the receipt claim, then every handover.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<UnitOwner>,
    /// Pending handover link created by the owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<Transfer>,
    /// Set when the code was seen in two countries at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<CloneFlag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitOwner {
    pub uid: String,
    pub name: String,
    pub at: String,
    /// "receipt" or "gift".
    pub via: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub token: String,
    pub from_uid: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneFlag {
    pub at: String,
    pub regions: Vec<String>,
}

/// One scan of a unit code, kept in `goods_scans/{batchId}.s_<code>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub at: String,
    pub region: String,
    /// Salted hash of IP + user agent: tells devices apart without storing either.
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub id: String,
    pub product_id: String,
    pub owner_id: String,
    pub mode: String,
    pub lot: String,
    #[serde(default)]
    pub produced_at: String,
    #[serde(default)]
    pub expires_at: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(default)]
    pub units: Vec<Unit>,
    #[serde(default)]
    pub events: Vec<BatchEvent>,
    /// Uids that claimed a unit, for "my goods" queries.
    #[serde(default)]
    pub claimed_by: Vec<String>,
    pub created_at: String,
}

impl Batch {
    /// Latest supply-chain stage reached, from the events.
    fn stage(&self) -> &str {
        self.events
            .iter()
            .rev()
            .find(|e| STAGES.contains(&e.kind.as_str()))
            .map(|e| e.kind.as_str())
            .unwrap_or("produced")
    }

    fn claimed(&self) -> usize {
        self.units.iter().filter(|u| u.owner_id.is_some()).count()
    }

    fn expired(&self, today: NaiveDate) -> bool {
        NaiveDate::parse_from_str(&self.expires_at, "%Y-%m-%d").is_ok_and(|d| d < today)
    }
}

// ── Requests / responses ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProductRequest {
    pub name: String,
    pub category: String,
    pub mode: String,
    #[serde(default)]
    pub description: String,
    /// Optional `data:image/...;base64,...` picture of the product.
    #[serde(default)]
    pub image_data: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBatchRequest {
    pub lot: String,
    #[serde(default)]
    pub produced_at: String,
    #[serde(default)]
    pub expires_at: String,
    /// Items with their own code; ignored for batch-mode products.
    #[serde(default)]
    pub units: u32,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(default)]
    pub location: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddEventRequest {
    pub kind: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewQuery {
    /// The viewer's IANA time zone (e.g. `Europe/Kyiv`), for the scan region.
    #[serde(default)]
    pub tz: Option<String>,
    /// Re-check without counting a scan (the page polls while open): `1` or `true`.
    #[serde(default)]
    pub peek: Option<String>,
}

impl ViewQuery {
    fn peek(&self) -> bool {
        matches!(self.peek.as_deref(), Some("1" | "true"))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptRequest {
    pub token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimRequest {
    /// Text of the fiscal receipt QR (a tax-office URL with fn/id/date/sm).
    pub receipt: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSummary {
    pub id: String,
    pub lot: String,
    pub produced_at: String,
    pub expires_at: String,
    pub size: usize,
    pub claimed: usize,
    pub stage: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductWithBatches {
    #[serde(flatten)]
    pub product: Product,
    pub batches: Vec<BatchSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitView {
    pub serial: u32,
    pub claimed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
    /// The signed-in viewer owns it (only on the authenticated endpoint).
    pub mine: bool,
    /// Owner names, oldest first (receipt buyer, then gift receivers).
    pub owners: Vec<UnitOwnerView>,
    /// The owner's pending handover token (only shown to the owner).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitOwnerView {
    pub name: String,
    pub at: String,
    pub via: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyView {
    pub code: String,
    pub mode: String,
    pub product: Product,
    pub brand_verified: bool,
    pub batch_id: String,
    pub lot: String,
    pub produced_at: String,
    pub expires_at: String,
    pub attributes: BTreeMap<String, String>,
    pub batch_size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<UnitView>,
    pub stage: String,
    pub events: Vec<BatchEvent>,
    /// Human-readable reasons to be careful, shown above everything else.
    pub warnings: Vec<String>,
    /// The code was seen in two countries at once: very likely a copied label.
    pub suspicious: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suspicious_regions: Vec<String>,
    /// How the item is protected: "qr" today, "nfc" for signed chips later.
    pub protection: String,
    pub scans: u64,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn short_id(len: usize) -> String {
    Uuid::new_v4().simple().to_string()[..len].to_owned()
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn fs_err(e: impl std::fmt::Display) -> AppError {
    AppError::Firebase(e.to_string())
}

async fn load<T: for<'de> Deserialize<'de>>(state: &AppState, collection: &str, id: &str, what: &str) -> ApiResult<T> {
    let doc = state
        .firestore
        .get(collection, id)
        .await
        .map_err(fs_err)?
        .ok_or_else(|| AppError::NotFound(format!("{what} not found")))?;
    serde_json::from_value(doc).map_err(|e| AppError::Internal(e.into()))
}

async fn save<T: Serialize>(state: &AppState, collection: &str, id: &str, value: &T) -> ApiResult<()> {
    let value = serde_json::to_value(value).map_err(|e| AppError::Internal(e.into()))?;
    state.firestore.set(collection, id, &value).await.map_err(fs_err)
}

async fn user_doc(state: &AppState, uid: &str) -> ApiResult<Value> {
    Ok(state.firestore.get("users", uid).await.map_err(fs_err)?.unwrap_or_else(|| json!({})))
}

fn display_name(user: &Value) -> String {
    let company = text(user, "companyName");
    if company.is_empty() { text(user, "name") } else { company }
}

fn clean(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

fn clean_attributes(attrs: BTreeMap<String, String>) -> BTreeMap<String, String> {
    attrs
        .into_iter()
        .map(|(k, v)| (clean(&k, 40), clean(&v, 200)))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .take(20)
        .collect()
}

fn valid_date(s: &str) -> bool {
    s.is_empty() || NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// Reads the receipt QR text. Ukrainian fiscal receipts encode a tax-office URL
/// whose query holds the register (`fn`), check number (`id`), `date` and sum
/// (`sm`); parameter order varies between registers.
pub fn parse_receipt(raw: &str) -> Option<Receipt> {
    let query = raw.trim().split_once('?').map(|(_, q)| q).unwrap_or(raw.trim());
    let mut params: BTreeMap<String, String> = BTreeMap::new();
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let v = urlencoding::decode(v).map(|c| c.into_owned()).unwrap_or_else(|_| v.to_owned());
            params.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let fiscal_number = params.get("fn").cloned().filter(|s| !s.is_empty())?;
    let check_id = params.get("id").cloned().filter(|s| !s.is_empty())?;
    Some(Receipt {
        fiscal_number,
        check_id,
        date: params.get("date").cloned().unwrap_or_default(),
        sum: params.get("sm").cloned().unwrap_or_default(),
    })
}

/// Receipt date as `YYYY-MM-DD` when it is in a known format (`YYYYMMDD` or ISO).
fn receipt_day(date: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(date, "%Y%m%d")
        .or_else(|_| NaiveDate::parse_from_str(date, "%Y-%m-%d"))
        .ok()
}

fn summary(b: &Batch) -> BatchSummary {
    BatchSummary {
        id: b.id.clone(),
        lot: b.lot.clone(),
        produced_at: b.produced_at.clone(),
        expires_at: b.expires_at.clone(),
        size: if b.mode == "unit" { b.units.len() } else { 0 },
        claimed: b.claimed(),
        stage: b.stage().to_owned(),
        created_at: b.created_at.clone(),
    }
}

/// Splits a public code into its batch id and (for units) the unit code.
fn split_code(code: &str) -> ApiResult<(String, Option<String>)> {
    let code = code.trim().to_ascii_lowercase();
    if !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::NotFound("Code not found".into()));
    }
    match code.len() {
        8 => Ok((code, None)),
        16 => Ok((code[..8].to_owned(), Some(code))),
        _ => Err(AppError::NotFound("Code not found".into())),
    }
}

/// Where a scan came from, as reported by the request.
pub struct ScanInput {
    pub region: String,
    pub device: String,
}

impl ScanInput {
    fn from_request(headers: &HeaderMap, tz: Option<&str>) -> Self {
        let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default();
        // Render puts the client address first in X-Forwarded-For.
        let ip = header("x-forwarded-for").split(',').next().unwrap_or_default().trim();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (SCAN_SALT.as_str(), ip, header("user-agent")).hash(&mut hasher);
        ScanInput { region: region_of(tz.unwrap_or_default()), device: format!("{:016x}", hasher.finish()) }
    }
}

/// Salt for device hashes, so they can't be matched to IPs outside this server.
static SCAN_SALT: LazyLock<String> =
    LazyLock::new(|| std::env::var("GOODS_SCAN_SALT").unwrap_or_else(|_| Uuid::new_v4().to_string()));

/// Coarse region of a scan: the country of the phone's time zone, else the zone itself.
pub fn region_of(tz: &str) -> String {
    let tz = tz.trim();
    let country = match tz {
        "Europe/Kyiv" | "Europe/Kiev" | "Europe/Uzhgorod" | "Europe/Zaporozhye" | "Europe/Simferopol" => "UA",
        "Europe/Madrid" | "Atlantic/Canary" | "Africa/Ceuta" => "ES",
        "Europe/Warsaw" => "PL",
        "Europe/Berlin" | "Europe/Busingen" => "DE",
        "Europe/Paris" => "FR",
        "Europe/Rome" => "IT",
        "Europe/Lisbon" | "Atlantic/Madeira" | "Atlantic/Azores" => "PT",
        "Europe/London" => "GB",
        "Europe/Prague" => "CZ",
        "Europe/Bratislava" => "SK",
        "Europe/Budapest" => "HU",
        "Europe/Bucharest" => "RO",
        "Europe/Chisinau" => "MD",
        "Europe/Vienna" => "AT",
        "Europe/Amsterdam" => "NL",
        "Europe/Brussels" => "BE",
        "Europe/Zurich" => "CH",
        "Europe/Istanbul" => "TR",
        "Europe/Vilnius" => "LT",
        "Europe/Riga" => "LV",
        "Europe/Tallinn" => "EE",
        "Europe/Tbilisi" | "Asia/Tbilisi" => "GE",
        "Europe/Athens" => "GR",
        "Europe/Sofia" => "BG",
        "Europe/Dublin" => "IE",
        "Europe/Stockholm" => "SE",
        "Europe/Oslo" => "NO",
        "Europe/Copenhagen" => "DK",
        "Europe/Helsinki" => "FI",
        "Asia/Dubai" => "AE",
        _ => "",
    };
    if !country.is_empty() {
        return country.to_owned();
    }
    let zone: String = tz.chars().filter(|c| c.is_ascii_alphanumeric() || "/_-".contains(*c)).take(40).collect();
    if zone.is_empty() { "?".into() } else { zone }
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

/// Uids whose scans never count as a clone: the owner, recent previous owners
/// and whoever holds a pending handover link's sender role.
fn trusted_uids(unit: &Unit, now: DateTime<Utc>) -> BTreeSet<String> {
    let mut trusted: BTreeSet<String> = unit.owner_id.iter().cloned().collect();
    let since = now - Duration::days(TRUSTED_AFTER_HANDOVER_DAYS);
    for (i, o) in unit.owners.iter().enumerate() {
        let handed_over_at = unit.owners.get(i + 1).and_then(|next| parse_time(&next.at));
        if handed_over_at.is_some_and(|t| t >= since) {
            trusted.insert(o.uid.clone());
        }
    }
    if let Some(t) = &unit.transfer {
        trusted.insert(t.from_uid.clone());
    }
    trusted
}

/// Regions seen within the clone window from at least two devices, ignoring
/// trusted people. Two or more regions means two copies of one label.
pub fn clone_regions(scans: &[Scan], trusted: &BTreeSet<String>, now: DateTime<Utc>) -> Vec<String> {
    let since = now - Duration::hours(CLONE_WINDOW_HOURS);
    let recent: Vec<&Scan> = scans
        .iter()
        .filter(|s| s.region != "?")
        .filter(|s| parse_time(&s.at).is_some_and(|t| t >= since))
        .filter(|s| s.uid.as_ref().is_none_or(|u| !trusted.contains(u)))
        .collect();
    let regions: BTreeSet<&str> = recent.iter().map(|s| s.region.as_str()).collect();
    let devices: BTreeSet<&str> = recent.iter().map(|s| s.device.as_str()).collect();
    if regions.len() >= 2 && devices.len() >= 2 {
        regions.into_iter().map(str::to_owned).collect()
    } else {
        Vec::new()
    }
}

/// Logs a scan of `code` and returns its total count and recent scans.
/// Failures never block a check.
async fn record_scan(state: &AppState, batch_id: &str, code: &str, scan: Option<Scan>) -> (u64, Vec<Scan>) {
    let doc = state.firestore.get(SCANS, batch_id).await.ok().flatten().unwrap_or_else(|| json!({}));
    let count_field = format!("c_{code}");
    let list_field = format!("s_{code}");
    let mut scans: Vec<Scan> = doc
        .get(&list_field)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let mut count = doc.get(&count_field).and_then(Value::as_u64).unwrap_or(0);
    if let Some(scan) = scan {
        count += 1;
        scans.push(scan);
        if scans.len() > MAX_SCANS_PER_UNIT {
            scans.drain(..scans.len() - MAX_SCANS_PER_UNIT);
        }
        let total = doc.get("total").and_then(Value::as_u64).unwrap_or(0) + 1;
        let mut fields = serde_json::Map::new();
        fields.insert(count_field, json!(count));
        fields.insert(list_field, json!(scans));
        fields.insert("total".into(), json!(total));
        let _ = state.firestore.update(SCANS, batch_id, &Value::Object(fields)).await;
    }
    (count, scans)
}

/// Marks a unit as a probable clone (once).
async fn flag_unit(state: &AppState, batch_id: &str, code: &str, regions: Vec<String>) -> ApiResult<()> {
    let _guard = BATCH_LOCK.lock().await;
    let mut batch: Batch = load(state, BATCHES, batch_id, "Batch").await?;
    if let Some(u) = batch.units.iter_mut().find(|u| u.code == code) {
        if u.flag.is_none() {
            u.flag = Some(CloneFlag { at: Utc::now().to_rfc3339(), regions });
            save(state, BATCHES, batch_id, &batch).await?;
        }
    }
    Ok(())
}

async fn build_view(state: &AppState, code: &str, viewer: Option<&str>, scan: Option<ScanInput>) -> ApiResult<VerifyView> {
    let (batch_id, unit_code) = split_code(code)?;
    let batch: Batch = load(state, BATCHES, &batch_id, "Code").await?;
    let product: Product = load(state, PRODUCTS, &batch.product_id, "Product").await?;
    let mut unit = match &unit_code {
        Some(c) => Some(
            batch
                .units
                .iter()
                .find(|u| &u.code == c)
                .cloned()
                .ok_or_else(|| AppError::NotFound("Code not found".into()))?,
        ),
        None if batch.mode == "unit" => return Err(AppError::NotFound("Code not found".into())),
        None => None,
    };

    let brand = user_doc(state, &batch.owner_id).await?;
    let brand_verified = brand.get("companyApproved").and_then(Value::as_bool).unwrap_or(false);

    let now = Utc::now();
    let code_lc = code.to_ascii_lowercase();
    let scan = scan.map(|s| Scan { at: now.to_rfc3339(), region: s.region, device: s.device, uid: viewer.map(str::to_owned) });
    let (scans, recent) = record_scan(state, &batch.id, &code_lc, scan).await;

    // Clone check: only unit codes; batch codes are on many packs by design.
    if let Some(u) = unit.as_mut() {
        if u.flag.is_none() {
            let regions = clone_regions(&recent, &trusted_uids(u, now), now);
            if !regions.is_empty() {
                let _ = flag_unit(state, &batch.id, &u.code, regions.clone()).await;
                u.flag = Some(CloneFlag { at: now.to_rfc3339(), regions });
            }
        }
    }

    let mut warnings = Vec::new();
    let mine = matches!((&unit, viewer), (Some(u), Some(v)) if u.owner_id.as_deref() == Some(v));
    let suspicious_regions = unit.as_ref().and_then(|u| u.flag.as_ref()).map(|f| f.regions.clone()).unwrap_or_default();
    if !suspicious_regions.is_empty() {
        warnings.push(format!(
            "Ймовірна підробка: цей код за добу сканували в різних країнах ({}). Справжня пляшка не може бути у двох місцях одночасно — хтось скопіював етикетку.",
            suspicious_regions.join(", ")
        ));
    }
    if let Some(u) = &unit {
        if u.owner_id.is_some() && !mine {
            warnings.push(format!(
                "Цей товар уже куплено й зареєстровано на покупця {}. Якщо вам продають його як новий — це підозріло.",
                u.claimed_at.as_deref().map(|d| d.get(..10).unwrap_or(d)).unwrap_or("раніше")
            ));
        }
    }
    if batch.expired(now.date_naive()) {
        warnings.push(format!("Термін придатності минув {}.", batch.expires_at));
    }
    if !brand_verified {
        warnings.push("Виробник ще не пройшов перевірку Marki.".into());
    }

    Ok(VerifyView {
        code: code_lc,
        mode: batch.mode.clone(),
        brand_verified,
        batch_id: batch.id.clone(),
        lot: batch.lot.clone(),
        produced_at: batch.produced_at.clone(),
        expires_at: batch.expires_at.clone(),
        attributes: batch.attributes.clone(),
        batch_size: batch.units.len(),
        unit: unit.map(|u| UnitView {
            serial: u.serial,
            claimed: u.owner_id.is_some(),
            claimed_at: u.claimed_at.clone(),
            mine,
            owners: u.owners.iter().map(|o| UnitOwnerView { name: o.name.clone(), at: o.at.clone(), via: o.via.clone() }).collect(),
            transfer_token: if mine { u.transfer.as_ref().map(|t| t.token.clone()) } else { None },
        }),
        stage: batch.stage().to_owned(),
        events: batch.events.clone(),
        suspicious: !suspicious_regions.is_empty(),
        suspicious_regions,
        protection: "qr".into(),
        warnings,
        scans,
        product,
    })
}

// ── Brand endpoints ─────────────────────────────────────────────────────────

/// `POST /api/goods/products`
pub async fn create_product(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(req): Json<CreateProductRequest>,
) -> ApiResult<(StatusCode, Json<Product>)> {
    let name = clean(&req.name, 120);
    if name.is_empty() {
        return Err(AppError::BadRequest("Name is required".into()));
    }
    if !CATEGORIES.contains(&req.category.as_str()) {
        return Err(AppError::BadRequest("Unknown category".into()));
    }
    if req.mode != "unit" && req.mode != "batch" {
        return Err(AppError::BadRequest("mode must be \"unit\" or \"batch\"".into()));
    }
    let me = user_doc(&state, &auth.uid).await?;
    let id = Uuid::new_v4().to_string();

    let image = match req.image_data.as_deref().filter(|s| !s.is_empty()) {
        Some(data) => {
            let (meta, b64) = data
                .split_once(";base64,")
                .ok_or_else(|| AppError::BadRequest("imageData must be a base64 data URL".into()))?;
            let content_type = meta.trim_start_matches("data:").to_owned();
            if !content_type.starts_with("image/") {
                return Err(AppError::BadRequest("imageData must be an image".into()));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|_| AppError::BadRequest("imageData is not valid base64".into()))?;
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(AppError::BadRequest("Image is larger than 2 MB".into()));
            }
            let ext = content_type.trim_start_matches("image/").replace("jpeg", "jpg");
            let path = format!("goods/{}/{}.{}", auth.uid, id, ext);
            state.storage.upload(&path, bytes, &content_type).await.map_err(fs_err)?
        }
        None => String::new(),
    };

    let product = Product {
        id: id.clone(),
        owner_id: auth.uid.clone(),
        brand_name: display_name(&me),
        name,
        category: req.category,
        mode: req.mode,
        description: clean(&req.description, 2_000),
        image,
        attributes: clean_attributes(req.attributes),
        created_at: Utc::now().to_rfc3339(),
    };
    save(&state, PRODUCTS, &id, &product).await?;
    Ok((StatusCode::CREATED, Json(product)))
}

/// `GET /api/goods/products` — the brand's products with their batches.
pub async fn my_products(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<ProductWithBatches>>> {
    let products = state
        .firestore
        .query(PRODUCTS, vec![QueryFilter::equal("ownerId", auth.uid.clone())], None, None)
        .await
        .map_err(fs_err)?;
    let batches = state
        .firestore
        .query(BATCHES, vec![QueryFilter::equal("ownerId", auth.uid.clone())], None, None)
        .await
        .map_err(fs_err)?;
    let batches: Vec<Batch> = batches.into_iter().filter_map(|b| serde_json::from_value(b).ok()).collect();

    let mut out: Vec<ProductWithBatches> = products
        .into_iter()
        .filter_map(|p| serde_json::from_value::<Product>(p).ok())
        .map(|product| {
            let mut list: Vec<BatchSummary> =
                batches.iter().filter(|b| b.product_id == product.id).map(summary).collect();
            list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            ProductWithBatches { product, batches: list }
        })
        .collect();
    out.sort_by(|a, b| b.product.created_at.cmp(&a.product.created_at));
    Ok(Json(out))
}

/// `POST /api/goods/products/:id/batches`
pub async fn create_batch(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(product_id): Path<String>,
    Json(req): Json<CreateBatchRequest>,
) -> ApiResult<(StatusCode, Json<Batch>)> {
    let product: Product = load(&state, PRODUCTS, &product_id, "Product").await?;
    if product.owner_id != auth.uid {
        return Err(AppError::Forbidden("Not your product".into()));
    }
    let lot = clean(&req.lot, 60);
    if lot.is_empty() {
        return Err(AppError::BadRequest("Lot number is required".into()));
    }
    if !valid_date(&req.produced_at) || !valid_date(&req.expires_at) {
        return Err(AppError::BadRequest("Dates must be YYYY-MM-DD".into()));
    }
    let count = if product.mode == "unit" {
        if req.units == 0 || req.units > MAX_UNITS {
            return Err(AppError::BadRequest(format!("units must be 1..={MAX_UNITS}")));
        }
        req.units
    } else {
        0
    };

    // An 8-char id is the public code of a batch-mode lot; retry on the rare clash.
    let mut id = short_id(8);
    for _ in 0..5 {
        if state.firestore.get(BATCHES, &id).await.map_err(fs_err)?.is_none() {
            break;
        }
        id = short_id(8);
    }

    let me = user_doc(&state, &auth.uid).await?;
    let now = Utc::now().to_rfc3339();
    let batch = Batch {
        units: (1..=count)
            .map(|serial| Unit {
                serial,
                code: format!("{id}{}", short_id(8)),
                owner_id: None,
                owner_name: None,
                claimed_at: None,
                receipt: None,
                owners: Vec::new(),
                transfer: None,
                flag: None,
            })
            .collect(),
        events: vec![BatchEvent {
            kind: "produced".into(),
            location: clean(&req.location, 120),
            note: String::new(),
            actor_name: display_name(&me),
            at: if req.produced_at.is_empty() { now.clone() } else { format!("{}T00:00:00Z", req.produced_at) },
        }],
        id: id.clone(),
        product_id,
        owner_id: auth.uid.clone(),
        mode: product.mode,
        lot,
        produced_at: req.produced_at,
        expires_at: req.expires_at,
        attributes: clean_attributes(req.attributes),
        claimed_by: Vec::new(),
        created_at: now,
    };
    save(&state, BATCHES, &id, &batch).await?;
    Ok((StatusCode::CREATED, Json(batch)))
}

/// `GET /api/goods/batches/:id` — full batch with unit codes (brand only).
pub async fn get_batch(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(batch_id): Path<String>,
) -> ApiResult<Json<Batch>> {
    let batch: Batch = load(&state, BATCHES, &batch_id, "Batch").await?;
    if batch.owner_id != auth.uid {
        return Err(AppError::Forbidden("Not your batch".into()));
    }
    Ok(Json(batch))
}

/// `POST /api/goods/batches/:id/events` — a supply-chain step for the whole batch.
pub async fn add_event(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(batch_id): Path<String>,
    Json(req): Json<AddEventRequest>,
) -> ApiResult<Json<Batch>> {
    if !EVENT_KINDS.contains(&req.kind.as_str()) {
        return Err(AppError::BadRequest("Unknown event kind".into()));
    }
    let me = user_doc(&state, &auth.uid).await?;
    let _guard = BATCH_LOCK.lock().await;
    let mut batch: Batch = load(&state, BATCHES, &batch_id, "Batch").await?;
    if batch.owner_id != auth.uid {
        return Err(AppError::Forbidden("Not your batch".into()));
    }
    if batch.events.len() >= 200 {
        return Err(AppError::BadRequest("Too many events on this batch".into()));
    }
    batch.events.push(BatchEvent {
        kind: req.kind,
        location: clean(&req.location, 120),
        note: clean(&req.note, 500),
        actor_name: display_name(&me),
        at: Utc::now().to_rfc3339(),
    });
    save(&state, BATCHES, &batch.id, &batch).await?;
    Ok(Json(batch))
}

// ── Public check, claim, my goods ───────────────────────────────────────────

/// `GET /api/public/goods/:code?tz=…` — what anyone sees after scanning.
pub async fn public_verify(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    Query(q): Query<ViewQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<VerifyView>> {
    let scan = (!q.peek()).then(|| ScanInput::from_request(&headers, q.tz.as_deref()));
    Ok(Json(build_view(&state, &code, None, scan).await?))
}

/// `GET /api/goods/verify/:code?tz=…` — the same for a signed-in viewer (knows `mine`).
pub async fn verify(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(code): Path<String>,
    Query(q): Query<ViewQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<VerifyView>> {
    let scan = (!q.peek()).then(|| ScanInput::from_request(&headers, q.tz.as_deref()));
    Ok(Json(build_view(&state, &code, Some(&auth.uid), scan).await?))
}

/// `POST /api/goods/verify/:code/claim` — the buyer registers the item with a receipt.
pub async fn claim(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(code): Path<String>,
    Json(req): Json<ClaimRequest>,
) -> ApiResult<Json<VerifyView>> {
    let receipt = parse_receipt(&req.receipt).ok_or_else(|| {
        AppError::BadRequest("Це не схоже на QR фіскального чека. Сфотографуйте QR-код унизу чека.".into())
    })?;
    let (batch_id, unit_code) = split_code(&code)?;
    let unit_code = unit_code.ok_or_else(|| AppError::BadRequest("This code is for a whole batch".into()))?;
    let me = user_doc(&state, &auth.uid).await?;
    let receipt_key = format!("{}:{}", receipt.fiscal_number, receipt.check_id).replace('/', "_");

    {
        let _guard = BATCH_LOCK.lock().await;
        let mut batch: Batch = load(&state, BATCHES, &batch_id, "Code").await?;
        let produced = NaiveDate::parse_from_str(&batch.produced_at, "%Y-%m-%d").ok();
        let unit = batch
            .units
            .iter_mut()
            .find(|u| u.code == unit_code)
            .ok_or_else(|| AppError::NotFound("Code not found".into()))?;
        if let Some(owner) = &unit.owner_id {
            return Err(if owner == &auth.uid {
                AppError::Conflict("Цей товар уже ваш".into())
            } else {
                AppError::Conflict("Цей товар уже зареєстрував інший покупець".into())
            });
        }
        if let (Some(day), Some(produced)) = (receipt_day(&receipt.date), produced) {
            if day < produced {
                return Err(AppError::BadRequest("Чек датований раніше, ніж вироблено товар".into()));
            }
        }

        let used = state.firestore.get(RECEIPTS, &receipt_key).await.map_err(fs_err)?.unwrap_or_else(|| json!({}));
        let mut codes: Vec<String> = used
            .get("codes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let receipt_owner = text(&used, "uid");
        if !receipt_owner.is_empty() && receipt_owner != auth.uid {
            return Err(AppError::Conflict("Цей чек уже використав інший покупець".into()));
        }
        if codes.len() >= MAX_UNITS_PER_RECEIPT {
            return Err(AppError::Conflict("За цим чеком зареєстровано забагато товарів".into()));
        }

        unit.owner_id = Some(auth.uid.clone());
        unit.owner_name = Some(text(&me, "name"));
        unit.claimed_at = Some(Utc::now().to_rfc3339());
        unit.receipt = Some(receipt.clone());
        unit.owners.push(UnitOwner {
            uid: auth.uid.clone(),
            name: text(&me, "name"),
            at: Utc::now().to_rfc3339(),
            via: "receipt".into(),
        });
        if !batch.claimed_by.contains(&auth.uid) {
            batch.claimed_by.push(auth.uid.clone());
        }
        save(&state, BATCHES, &batch.id, &batch).await?;

        codes.push(unit_code.clone());
        state
            .firestore
            .set(RECEIPTS, &receipt_key, &json!({ "uid": auth.uid, "codes": codes, "updatedAt": Utc::now().to_rfc3339() }))
            .await
            .map_err(fs_err)?;
    }

    Ok(Json(build_view(&state, &unit_code, Some(&auth.uid), None).await?))
}

/// `GET /api/goods/mine` — items the signed-in user registered.
pub async fn my_goods(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<VerifyView>>> {
    let batches = state
        .firestore
        .query(BATCHES, vec![QueryFilter::array_contains("claimedBy", auth.uid.clone())], None, None)
        .await
        .map_err(fs_err)?;
    let mut out = Vec::new();
    for b in batches.into_iter().filter_map(|b| serde_json::from_value::<Batch>(b).ok()) {
        for u in b.units.iter().filter(|u| u.owner_id.as_deref() == Some(auth.uid.as_str())) {
            if let Ok(view) = build_view(&state, &u.code, Some(&auth.uid), None).await {
                out.push(view);
            }
        }
    }
    out.sort_by(|a, b| {
        let at = |v: &VerifyView| v.unit.as_ref().and_then(|u| u.claimed_at.clone()).unwrap_or_default();
        at(b).cmp(&at(a))
    });
    Ok(Json(out))
}

/// Loads a batch and the unit behind `code` for a write (call under `BATCH_LOCK`).
async fn load_unit(state: &AppState, code: &str) -> ApiResult<(Batch, usize)> {
    let (batch_id, unit_code) = split_code(code)?;
    let unit_code = unit_code.ok_or_else(|| AppError::BadRequest("This code is for a whole batch".into()))?;
    let batch: Batch = load(state, BATCHES, &batch_id, "Code").await?;
    let idx = batch
        .units
        .iter()
        .position(|u| u.code == unit_code)
        .ok_or_else(|| AppError::NotFound("Code not found".into()))?;
    Ok((batch, idx))
}

/// `POST /api/goods/verify/:code/transfer` — the owner creates a handover link (gift).
pub async fn create_transfer(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(code): Path<String>,
) -> ApiResult<Json<Value>> {
    let _guard = BATCH_LOCK.lock().await;
    let (mut batch, idx) = load_unit(&state, &code).await?;
    let unit = &mut batch.units[idx];
    if unit.owner_id.as_deref() != Some(auth.uid.as_str()) {
        return Err(AppError::Forbidden("Only the owner can hand this item over".into()));
    }
    let token = short_id(12);
    unit.transfer = Some(Transfer { token: token.clone(), from_uid: auth.uid.clone(), created_at: Utc::now().to_rfc3339() });
    save(&state, BATCHES, &batch.id, &batch).await?;
    Ok(Json(json!({ "token": token })))
}

/// `DELETE /api/goods/verify/:code/transfer` — cancels the handover link.
pub async fn cancel_transfer(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(code): Path<String>,
) -> ApiResult<StatusCode> {
    let _guard = BATCH_LOCK.lock().await;
    let (mut batch, idx) = load_unit(&state, &code).await?;
    let unit = &mut batch.units[idx];
    if unit.owner_id.as_deref() != Some(auth.uid.as_str()) {
        return Err(AppError::Forbidden("Not your item".into()));
    }
    unit.transfer = None;
    save(&state, BATCHES, &batch.id, &batch).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/goods/verify/:code/accept` — the receiver takes the item over.
pub async fn accept_transfer(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path(code): Path<String>,
    Json(req): Json<AcceptRequest>,
) -> ApiResult<Json<VerifyView>> {
    let me = user_doc(&state, &auth.uid).await?;
    let unit_code = {
        let _guard = BATCH_LOCK.lock().await;
        let (mut batch, idx) = load_unit(&state, &code).await?;
        let unit = &mut batch.units[idx];
        let transfer = unit
            .transfer
            .clone()
            .filter(|t| t.token == req.token.trim())
            .ok_or_else(|| AppError::NotFound("Посилання для передачі недійсне або вже використане".into()))?;
        if transfer.from_uid == auth.uid {
            return Err(AppError::Conflict("Це ваш товар — надішліть посилання тому, кому даруєте".into()));
        }
        let now = Utc::now().to_rfc3339();
        if unit.owners.is_empty() {
            // Claimed before the chain existed: start it with the receipt owner.
            unit.owners.push(UnitOwner {
                uid: transfer.from_uid.clone(),
                name: unit.owner_name.clone().unwrap_or_default(),
                at: unit.claimed_at.clone().unwrap_or_else(|| now.clone()),
                via: "receipt".into(),
            });
        }
        unit.owners.push(UnitOwner { uid: auth.uid.clone(), name: text(&me, "name"), at: now, via: "gift".into() });
        unit.owner_id = Some(auth.uid.clone());
        unit.owner_name = Some(text(&me, "name"));
        unit.transfer = None;
        let unit_code = unit.code.clone();
        if !batch.claimed_by.contains(&auth.uid) {
            batch.claimed_by.push(auth.uid.clone());
        }
        save(&state, BATCHES, &batch.id, &batch).await?;
        unit_code
    };
    Ok(Json(build_view(&state, &unit_code, Some(&auth.uid), None).await?))
}

/// `DELETE /api/goods/batches/:id/units/:serial/flag` — the brand clears a false alarm.
pub async fn clear_flag(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Path((batch_id, serial)): Path<(String, u32)>,
) -> ApiResult<Json<Batch>> {
    let _guard = BATCH_LOCK.lock().await;
    let mut batch: Batch = load(&state, BATCHES, &batch_id, "Batch").await?;
    if batch.owner_id != auth.uid {
        return Err(AppError::Forbidden("Not your batch".into()));
    }
    let unit = batch
        .units
        .iter_mut()
        .find(|u| u.serial == serial)
        .ok_or_else(|| AppError::NotFound("Unit not found".into()))?;
    unit.flag = None;
    let code = unit.code.clone();
    save(&state, BATCHES, &batch.id, &batch).await?;
    // Forget the scans that raised it, or the next scan flags it again.
    let _ = state.firestore.update(SCANS, &batch.id, &json!({ format!("s_{code}"): [] })).await;
    Ok(Json(batch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tax_office_receipt_urls() {
        let r = parse_receipt("https://cabinet.tax.gov.ua/cashregs/check?id=123456&fn=4000123456&date=20261012&time=134501&sm=459.90").unwrap();
        assert_eq!((r.fiscal_number.as_str(), r.check_id.as_str(), r.date.as_str(), r.sum.as_str()), ("4000123456", "123456", "20261012", "459.90"));
        // Parameter order and case vary between registers.
        let r = parse_receipt("https://cabinet.tax.gov.ua/cashregs/check?FN=1&ID=2").unwrap();
        assert_eq!((r.fiscal_number.as_str(), r.check_id.as_str()), ("1", "2"));
        assert!(parse_receipt("https://example.com/?foo=bar").is_none());
        assert!(parse_receipt("just text").is_none());
        assert_eq!(receipt_day("20261012"), NaiveDate::from_ymd_opt(2026, 10, 12));
    }

    #[test]
    fn codes_split_into_batch_and_unit() {
        assert_eq!(split_code("ABCDEF12").unwrap(), ("abcdef12".into(), None));
        assert_eq!(split_code("abcdef1200000000").unwrap(), ("abcdef12".into(), Some("abcdef1200000000".into())));
        assert!(split_code("abc").is_err());
        assert!(split_code("../etc/passwd").is_err());
    }

    #[test]
    fn stage_follows_supply_chain_events() {
        let ev = |kind: &str| BatchEvent { kind: kind.into(), location: String::new(), note: String::new(), actor_name: String::new(), at: String::new() };
        let mut b = Batch {
            id: "b".into(), product_id: "p".into(), owner_id: "o".into(), mode: "unit".into(), lot: "L1".into(),
            produced_at: String::new(), expires_at: "2026-01-01".into(), attributes: BTreeMap::new(),
            units: vec![], events: vec![ev("produced"), ev("shipped"), ev("note")], claimed_by: vec![], created_at: String::new(),
        };
        assert_eq!(b.stage(), "shipped");
        b.events.push(ev("in_store"));
        assert_eq!(b.stage(), "in_store");
        assert!(b.expired(NaiveDate::from_ymd_opt(2026, 10, 12).unwrap()));
    }

    fn scan(hours_ago: i64, region: &str, device: &str, uid: Option<&str>) -> Scan {
        Scan { at: (Utc::now() - Duration::hours(hours_ago)).to_rfc3339(), region: region.into(), device: device.into(), uid: uid.map(Into::into) }
    }

    #[test]
    fn two_countries_in_a_day_flag_a_clone() {
        let none = BTreeSet::new();
        let now = Utc::now();
        // Valencia and Ukraine within hours, different phones → clone.
        assert_eq!(clone_regions(&[scan(3, "ES", "a", None), scan(1, "UA", "b", None)], &none, now), ["ES", "UA"]);
        // Same phone travelling, or one country, or two days apart → fine.
        assert!(clone_regions(&[scan(3, "ES", "a", None), scan(1, "UA", "a", None)], &none, now).is_empty());
        assert!(clone_regions(&[scan(3, "UA", "a", None), scan(1, "UA", "b", None)], &none, now).is_empty());
        assert!(clone_regions(&[scan(50, "ES", "a", None), scan(1, "UA", "b", None)], &none, now).is_empty());
        // The owner (or a gift receiver) scanning abroad is trusted.
        let trusted: BTreeSet<String> = ["owner".to_string()].into();
        assert!(clone_regions(&[scan(3, "ES", "a", Some("owner")), scan(1, "UA", "b", None)], &trusted, now).is_empty());
    }

    #[test]
    fn time_zones_map_to_countries() {
        assert_eq!(region_of("Europe/Madrid"), "ES");
        assert_eq!(region_of("Europe/Kiev"), region_of("Europe/Kyiv"));
        assert_eq!(region_of("America/New_York"), "America/New_York");
        assert_eq!(region_of(""), "?");
    }

    #[test]
    fn peek_accepts_the_forms_browsers_send() {
        let q = |p: Option<&str>| ViewQuery { tz: None, peek: p.map(Into::into) };
        assert!(q(Some("1")).peek() && q(Some("true")).peek());
        assert!(!q(None).peek() && !q(Some("0")).peek());
    }
}
