//! Card payments for plans (WayForPay), test mode first.
//!
//! The app asks for a checkout: the server records an order and signs the
//! WayForPay form; the buyer pays on WayForPay's page. WayForPay then posts the
//! result to `/api/public/billing/wayforpay`; the server checks the HMAC-MD5
//! signature, logs every step on the order and, once approved, activates the plan
//! in the canister for the buyer's wallet (signed with the Marki server key).
//!
//! Env: `WFP_MERCHANT`, `WFP_SECRET` (default: WayForPay's public test merchant),
//! `WFP_DOMAIN`, `BILLING_PRICE` + `BILLING_CURRENCY` (default 0.01 USD while testing), `PUBLIC_API_URL`.

use axum::{body::Bytes, extract::{Path, State}, response::{Html, IntoResponse, Redirect, Response}, Extension, Json};
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
fn price() -> String { env("BILLING_PRICE", "0.01") }
fn currency() -> String { env("BILLING_CURRENCY", "USD") }

/// `GET /api/public/billing/price` — what a plan costs by card right now (the test price while testing).
pub async fn current_price() -> Json<Value> {
    Json(json!({ "amount": price(), "currency": currency(), "testMode": test_mode() }))
}
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
    /// Paid from the phone app (see `pay_page` and `back_to_app`).
    #[serde(default)]
    pub app: bool,
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
    let cur = currency();
    let signature = sign(&[&merchant, &domain, &id, &date, &amount, &cur, &product, "1", &amount], &secret());
    let api = env("PUBLIC_API_URL", "https://idenity.onrender.com");

    let fields = vec![
        ("merchantAccount".into(), merchant),
        ("merchantAuthType".into(), "SimpleSignature".into()),
        ("merchantDomainName".into(), domain),
        ("merchantTransactionSecureType".into(), "AUTO".into()),
        ("merchantSignature".into(), signature),
        ("orderReference".into(), id.clone()),
        ("orderDate".into(), date),
        ("amount".into(), amount.clone()),
        ("currency".into(), cur.clone()),
        ("productName[]".into(), product),
        ("productCount[]".into(), "1".into()),
        ("productPrice[]".into(), amount.clone()),
        ("serviceUrl".into(), format!("{api}/api/public/billing/wayforpay")),
        ("language".into(), "UA".into()),
        // WayForPay POSTs the buyer back; static sites refuse POST, so bounce via the API.
        ("returnUrl".into(), format!("{api}/api/public/billing/return/{id}")),
    ];
    let mut order = json!({
        "id": id, "uid": auth.uid, "planId": plan_id, "principal": req.principal.trim(),
        "amount": amount, "currency": cur, "status": "pending", "credited": false,
        "testMode": test_mode(), "createdAt": Utc::now().to_rfc3339(),
        "returnTo": req.return_url.as_deref().filter(|u| u.starts_with("https://")),
        // Paid from the phone app: WayForPay's page opens in a browser tab, which then hands back to the app.
        "app": req.app,
        "form": fields.iter().map(|(n, v)| json!({ "n": n, "v": v })).collect::<Vec<_>>(),
    });
    log_line(&mut order, format!("Створено замовлення: тариф «{plan_name}», {amount} {cur}{}", if test_mode() { " (тестовий режим)" } else { "" }));
    state.firestore.set(ORDERS, &id, &order).await.map_err(fs)?;

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
    let reason = match (field("reason"), field("reasonCode")) {
        (r, c) if r.is_empty() && c.is_empty() => String::new(),
        (r, c) => format!(" · причина: {r} ({c})"),
    };
    log_line(&mut order, format!("WayForPay: {status} · картка {} · {} {}{reason}", field("cardPan"), field("amount"), field("currency")));
    order["status"] = json!(status.to_lowercase());

    if status == "Approved" && order["credited"] != json!(true) {
        let principal = order["principal"].as_str().unwrap_or_default().to_owned();
        let plan = order["planId"].as_str().unwrap_or_default().to_owned();
        match (crate::services::icp::custody().await, candid::Principal::from_text(&principal)) {
            (Some(c), Ok(owner)) => match c.grant_plan(owner, &plan).await {
                Ok(Ok(_)) => {
                    order["credited"] = json!(true);
                    let name = PLANS.iter().find(|(id, _)| *id == plan).map_or(plan.as_str(), |(_, n)| n);
                    log_line(&mut order, format!("Тариф «{name}» активовано в канистрі для {principal}"));
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

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(r#"<!doctype html><html lang="uk"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title><style>body{{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;background:#0b0f14;color:#e6edf5;font:16px system-ui,sans-serif;text-align:center}}
main{{padding:24px;max-width:420px}}a,button{{display:block;margin:18px auto 0;padding:14px 22px;border-radius:12px;border:0;background:#4f8cff;color:#fff;font:600 16px system-ui;text-decoration:none}}p{{color:#8a99ad}}</style></head>
<body><main>{body}</main></body></html>"#))
}

/// `GET /api/public/billing/pay/:id` — WayForPay's payment form for an app order,
/// so the app can open the payment as a plain link in a browser tab.
pub async fn pay_page(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Html<String> {
    let order = state.firestore.get(ORDERS, &id).await.ok().flatten();
    let Some(order) = order.filter(|o| o["status"] == "pending") else {
        return page("Marki", "<h2>Замовлення не знайдено</h2><p>Поверніться в застосунок і спробуйте ще раз.</p>");
    };
    let inputs: String = order["form"].as_array().into_iter().flatten()
        .map(|f| format!(r#"<input type="hidden" name="{}" value="{}">"#, esc(f["n"].as_str().unwrap_or_default()), esc(f["v"].as_str().unwrap_or_default())))
        .collect();
    page("Оплата · Marki", &format!(r#"<h2>Переходимо до оплати…</h2><form id="f" method="post" action="https://secure.wayforpay.com/pay" accept-charset="utf-8">{inputs}<button>Оплатити</button></form>
<script>document.getElementById('f').submit()</script>"#))
}

/// `GET|POST /api/public/billing/return/:id` — after the payment: back to the app
/// for app orders (with the website as fallback), otherwise to the web page paid from.
pub async fn back_to_app(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let order = state.firestore.get(ORDERS, &id).await.ok().flatten();
    let web = order.as_ref().and_then(|o| o["returnTo"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "https://borisven.github.io/MARKI_ICP/".into());
    let sep = if web.contains('?') { '&' } else { '?' };
    let web = format!("{web}{sep}paid={}", urlencoding::encode(&id));
    if order.as_ref().is_some_and(|o| o["app"] == true) {
        let intent = format!(
            "intent://paid?order={}#Intent;scheme=marki;package=com.identity.app;S.browser_fallback_url={};end",
            urlencoding::encode(&id), urlencoding::encode(&web),
        );
        return page("Marki", &format!(r#"<h2>Оплату оброблено ✅</h2><p>Результат уже в застосунку Marki.</p><a href="{0}">Повернутися в Marki</a>
<script>location.href={1:?}</script>"#, esc(&intent), intent)).into_response();
    }
    Redirect::to(&web).into_response()
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

#[cfg(test)]
mod live {
    use super::*;
    use crate::services::{FirestoreClient, GoogleAuthService};

    /// Live, test merchant only: sends production a signed "Approved" notice for
    /// the newest pending order, as WayForPay would after a successful payment.
    /// `cargo test approve_latest_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn approve_latest_live() {
        dotenvy::dotenv().ok();
        assert!(test_mode(), "only for the test merchant");
        let config = crate::config::Config::from_env().unwrap();
        let db = FirestoreClient::new(config.clone(), Arc::new(GoogleAuthService::new(config).unwrap()));
        // `BILLING_ORDER=<id>` repeats the notice for that order (e.g. once the server key is set).
        let o = match std::env::var("BILLING_ORDER") {
            Ok(id) => db.get(ORDERS, &id).await.unwrap().expect("no such order"),
            Err(_) => {
                let mut orders = db.query(ORDERS, vec![QueryFilter::equal("status", "pending")], None, None).await.unwrap();
                orders.sort_by(|a, b| b["createdAt"].as_str().cmp(&a["createdAt"].as_str()));
                orders.into_iter().next().expect("no pending order")
            }
        };
        let o = &o;
        let id = o["id"].as_str().unwrap();
        println!("order {id} · plan {} · {} · {}", o["planId"], o["principal"], o["createdAt"]);
        let (amount, pan, status, reason, auth) = (o["amount"].as_str().unwrap(), "41****1111", "Approved", "1100", "541963");
        let cur = o["currency"].as_str().unwrap_or("UAH");
        let sig = sign(&[&merchant(), id, amount, cur, auth, pan, status, reason], &secret());
        let notice = json!({
            "merchantAccount": merchant(), "orderReference": id, "merchantSignature": sig,
            "amount": serde_json::from_str::<Value>(amount).unwrap(), "currency": cur, "authCode": auth, "cardPan": pan,
            "transactionStatus": status, "reasonCode": reason.parse::<u32>().unwrap(), "reason": "Ok",
        });
        let r = reqwest::Client::new().post("https://idenity.onrender.com/api/public/billing/wayforpay")
            .json(&notice).send().await.unwrap();
        println!("server: {} {}", r.status(), r.text().await.unwrap());
        let after = db.get(ORDERS, id).await.unwrap().unwrap();
        println!("credited: {}", after["credited"]);
        for l in after["log"].as_array().unwrap() { println!("  {}", l["text"]); }
    }
}

#[cfg(test)]
mod orders_live {
    use super::*;
    use crate::services::{FirestoreClient, GoogleAuthService};

    /// Live: card orders of `BILLING_UID`. `cargo test orders_of_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn orders_of_live() {
        dotenvy::dotenv().ok();
        let config = crate::config::Config::from_env().unwrap();
        let db = FirestoreClient::new(config.clone(), Arc::new(GoogleAuthService::new(config).unwrap()));
        let orders = match std::env::var("BILLING_ORDER") {
            Ok(id) => db.get(ORDERS, &id).await.unwrap().into_iter().collect(),
            Err(_) => db.query(ORDERS, vec![QueryFilter::equal("uid", std::env::var("BILLING_UID").unwrap())], None, None).await.unwrap(),
        };
        for o in orders {
            println!("{} {} {} credited={} principal={} return={}", o["createdAt"], o["id"], o["status"], o["credited"], o["principal"], o["returnTo"]);
            for l in o["log"].as_array().into_iter().flatten() { println!("   {}", l["text"]); }
        }
    }
}
