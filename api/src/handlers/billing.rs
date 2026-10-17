//! Card payments for plans (WayForPay), test mode first.
//!
//! The app asks for a checkout: the server records an order and signs the
//! WayForPay form; the buyer pays on WayForPay's page. WayForPay then posts the
//! result to `/api/public/billing/wayforpay`; the server checks the HMAC-MD5
//! signature, logs every step on the order and, once approved, activates the plan
//! in the canister for the buyer's wallet (signed with the Marki server key).
//!
//! Env: `WFP_MERCHANT`, `WFP_SECRET` (default: WayForPay's public test merchant),
//! `WFP_DOMAIN`, `BILLING_PRICE_UAH` (default 1 while testing), `PUBLIC_API_URL`.

use axum::{body::Bytes, extract::{Path, State}, response::Redirect, Extension, Json};
use chrono::Utc;
use hmac::{Hmac, Mac};
use md5::Md5;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    errors::{ApiResult, AppError},
    middleware::auth::AuthenticatedUser,
    services::firestore::QueryFilter,
    AppState,
};

const ORDERS: &str = "billing_orders";
const PLANS: &[(&str, &str)] = &[("start", "Старт"), ("business", "Бізнес"), ("pro", "Про")];

fn env(key: &str, default: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_owned())
}
fn merchant() -> String { env("WFP_MERCHANT", "test_merch_n1") }
fn secret() -> String { env("WFP_SECRET", "flk3409refn54t54t*FNJRET") }
fn domain() -> String { env("WFP_DOMAIN", "borisven.github.io") }
fn price() -> String { env("BILLING_PRICE_UAH", "1") }
pub fn test_mode() -> bool { merchant() == "test_merch_n1" }

/// HMAC-MD5 (hex) of the fields joined with ";" — WayForPay's signature.
pub fn sign(parts: &[&str], key: &str) -> String {
    let mut mac = Hmac::<Md5>::new_from_slice(key.as_bytes()).expect("any key length");
    mac.update(parts.join(";").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn fs(e: impl std::fmt::Display) -> AppError {
    AppError::Firebase(e.to_string())
}

/// Appends a timestamped line to an order's log (also to the server log).
fn log_line(order: &mut Value, line: String) {
    tracing::info!(order = %order["id"], "{line}");
    let entry = json!({ "at": Utc::now().to_rfc3339(), "text": line });
    match order.get_mut("log").and_then(Value::as_array_mut) {
        Some(log) => log.push(entry),
        None => order["log"] = json!([entry]),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutRequest {
    pub plan_id: String,
    /// The wallet the plan is for (the one that mints).
    pub principal: String,
    /// Where WayForPay sends the buyer back.
    #[serde(default)]
    pub return_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkout {
    pub order_id: String,
    pub url: String,
    /// Form fields to POST to `url` (array fields end with `[]`).
    pub fields: Vec<(String, String)>,
    pub test_mode: bool,
}

/// `POST /api/billing/checkout` — create an order and a signed WayForPay form.
pub async fn checkout(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
    Json(req): Json<CheckoutRequest>,
) -> ApiResult<Json<Checkout>> {
    let (plan_id, plan_name) = PLANS.iter().find(|(id, _)| *id == req.plan_id)
        .ok_or_else(|| AppError::BadRequest("Невідомий тариф".into()))?;
    candid::Principal::from_text(req.principal.trim())
        .map_err(|_| AppError::BadRequest("Підключіть гаманець, на який активувати тариф".into()))?;

    let id = format!("MK-{}", &Uuid::new_v4().simple().to_string()[..12]);
    let date = Utc::now().timestamp().to_string();
    let amount = price();
    let product = format!("Marki · тариф «{plan_name}»");
    let (merchant, domain) = (merchant(), domain());
    let signature = sign(&[&merchant, &domain, &id, &date, &amount, "UAH", &product, "1", &amount], &secret());
    let api = env("PUBLIC_API_URL", "https://idenity.onrender.com");

    let mut order = json!({
        "id": id, "uid": auth.uid, "planId": plan_id, "principal": req.principal.trim(),
        "amount": amount, "currency": "UAH", "status": "pending", "credited": false,
        "testMode": test_mode(), "createdAt": Utc::now().to_rfc3339(),
        "returnTo": req.return_url.as_deref().filter(|u| u.starts_with("https://")),
    });
    log_line(&mut order, format!("Створено замовлення: тариф «{plan_name}», {amount} UAH{}", if test_mode() { " (тестовий режим)" } else { "" }));
    state.firestore.set(ORDERS, &id, &order).await.map_err(fs)?;

    let fields = vec![
        ("merchantAccount".into(), merchant),
        ("merchantAuthType".into(), "SimpleSignature".into()),
        ("merchantDomainName".into(), domain),
        ("merchantTransactionSecureType".into(), "AUTO".into()),
        ("merchantSignature".into(), signature),
        ("orderReference".into(), id.clone()),
        ("orderDate".into(), date),
        ("amount".into(), amount.clone()),
        ("currency".into(), "UAH".into()),
        ("productName[]".into(), product),
        ("productCount[]".into(), "1".into()),
        ("productPrice[]".into(), amount),
        ("serviceUrl".into(), format!("{api}/api/public/billing/wayforpay")),
        ("language".into(), "UA".into()),
        // WayForPay POSTs the buyer back; static sites refuse POST, so bounce via the API.
        ("returnUrl".into(), format!("{api}/api/public/billing/return/{id}")),
    ];
    Ok(Json(Checkout { order_id: id, url: "https://secure.wayforpay.com/pay".into(), fields, test_mode: test_mode() }))
}

/// WayForPay posts JSON, sometimes form-encoded as the only key: take the JSON object.
fn parse_notice(body: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(body);
    let text = urlencoding::decode(&text).map(|t| t.into_owned()).unwrap_or_else(|_| text.into_owned());
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    serde_json::from_str(&text[start..=end]).ok()
}

/// `POST /api/public/billing/wayforpay` — payment result from WayForPay.
pub async fn wayforpay_notice(State(state): State<Arc<AppState>>, body: Bytes) -> ApiResult<Json<Value>> {
    let n = parse_notice(&body).ok_or_else(|| AppError::BadRequest("unreadable notice".into()))?;
    let field = |k: &str| match n.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(x)) => x.to_string(),
        _ => String::new(),
    };
    let id = field("orderReference");
    let expected = sign(&[
        &field("merchantAccount"), &id, &field("amount"), &field("currency"),
        &field("authCode"), &field("cardPan"), &field("transactionStatus"), &field("reasonCode"),
    ], &secret());
    if expected != field("merchantSignature") {
        tracing::warn!(order = %id, "WayForPay notice with a bad signature ignored");
        return Err(AppError::Unauthorized("bad signature".into()));
    }

    let mut order = state.firestore.get(ORDERS, &id).await.map_err(fs)?
        .ok_or_else(|| AppError::NotFound("order not found".into()))?;
    let status = field("transactionStatus");
    log_line(&mut order, format!("WayForPay: {status} · картка {} · {} {}", field("cardPan"), field("amount"), field("currency")));
    order["status"] = json!(status.to_lowercase());

    if status == "Approved" && order["credited"] != json!(true) {
        let principal = order["principal"].as_str().unwrap_or_default().to_owned();
        let plan = order["planId"].as_str().unwrap_or_default().to_owned();
        match (crate::services::icp::custody().await, candid::Principal::from_text(&principal)) {
            (Some(c), Ok(owner)) => match c.grant_plan(owner, &plan).await {
                Ok(Ok(_)) => {
                    order["credited"] = json!(true);
                    log_line(&mut order, format!("Тариф «{plan}» активовано в канистрі для {principal}"));
                }
                Ok(Err(e)) => log_line(&mut order, format!("Канистра відмовила: {e}")),
                Err(e) => log_line(&mut order, format!("Не вдалося звернутися до канистри: {e}")),
            },
            _ => log_line(&mut order, "Оплату отримано, але ключ сервера не налаштований — тариф не активовано".into()),
        }
    }
    state.firestore.set(ORDERS, &id, &order).await.map_err(fs)?;

    let time = Utc::now().timestamp();
    let reply_sig = sign(&[&id, "accept", &time.to_string()], &secret());
    Ok(Json(json!({ "orderReference": id, "status": "accept", "time": time, "signature": reply_sig })))
}

/// `GET|POST /api/public/billing/return/:id` — send the buyer back to the app page they paid from.
pub async fn back_to_app(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Redirect {
    let to = state.firestore.get(ORDERS, &id).await.ok().flatten()
        .and_then(|o| o["returnTo"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "https://borisven.github.io/MARKI_ICP/".into());
    let sep = if to.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{to}{sep}paid={}", urlencoding::encode(&id)))
}

/// `GET /api/billing/orders` — my card orders with their logs, newest first.
pub async fn my_orders(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedUser>,
) -> ApiResult<Json<Vec<Value>>> {
    let mut orders = state.firestore.query(ORDERS, vec![QueryFilter::equal("uid", auth.uid.clone())], None, None)
        .await.map_err(fs)?;
    orders.sort_by(|a, b| b["createdAt"].as_str().cmp(&a["createdAt"].as_str()));
    Ok(Json(orders))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_wayforpay_docs_example_shape() {
        // HMAC-MD5 of the ";"-joined fields, lowercase hex, 32 chars.
        let s = sign(&["test_merch_n1", "www.market.ua", "DH783023", "1415379863", "1547.36", "UAH"], "flk3409refn54t54t*FNJRET");
        assert_eq!(s.len(), 32);
        assert_eq!(s, sign(&["test_merch_n1", "www.market.ua", "DH783023", "1415379863", "1547.36", "UAH"], "flk3409refn54t54t*FNJRET"));
        assert_ne!(s, sign(&["test_merch_n1"], "flk3409refn54t54t*FNJRET"));
    }

    #[test]
    fn notices_parse_as_json_or_form_wrapped_json() {
        assert_eq!(parse_notice(br#"{"orderReference":"MK-1"}"#).unwrap()["orderReference"], "MK-1");
        assert_eq!(parse_notice(b"%7B%22orderReference%22%3A%22MK-2%22%7D=").unwrap()["orderReference"], "MK-2");
    }
}
