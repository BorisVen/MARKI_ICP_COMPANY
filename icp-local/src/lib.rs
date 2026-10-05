//! MarkICP payments test canister.
//!
//! Flow (ICRC-2, the same pattern the production backend should use):
//! 1. Seller calls `create_listing(nft_id, price_e8s)`.
//! 2. Buyer calls `icrc2_approve` on the ICP ledger with this canister as spender
//!    for `price_e8s + ledger fee`.
//! 3. Buyer calls `buy(listing_id)`. The canister pulls `price_e8s` from the buyer
//!    with `icrc2_transfer_from`, keeps the platform fee and pays the seller the rest.
//!
//! Plans: a company buys a plan (N photo mints + M metadata mints for 30 days)
//! with `icrc2_approve` + `buy_plan`. Plan prices are in USD and converted to ICP
//! with the ICP/XDR rate of the cycles minting canister (CMC). The backend spends
//! credits with `consume_mints` before each mint.
//!
//! Cost model: `quote_mint` prices a mint from the cycles measured in
//! `icp-mint-cost-test`. Use it to check plan margins.
//!
//! NFTs: ICRC-7 subset. `mint_uploaded` / `mint_metadata` take plan credits and
//! mint in the same call; images are stored in the canister.
//!
//! State lives in heap memory: this canister is for local testing only.

use candid::{CandidType, Nat, Principal};
use ic_cdk::call::Call;
use ic_cdk::{query, update};
use icrc_ledger_types::icrc1::account::Account;
use icrc_ledger_types::icrc1::transfer::{TransferArg, TransferError};
use icrc_ledger_types::icrc2::transfer_from::{TransferFromArgs, TransferFromError};
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::BTreeMap;

const ICP_LEDGER: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";
const LEDGER_FEE_E8S: u64 = 10_000;
/// 250 basis points = 2.5 %.
const PLATFORM_FEE_BPS: u16 = 250;

const CMC: &str = "rkp4c-7iaaa-aaaaa-aaaca-cai";
/// Refresh the ICP/XDR rate when the cached one is older than this.
const RATE_MAX_AGE_NS: u64 = 10 * 60 * 1_000_000_000;

// Cycles model, measured on the local network in icp-mint-cost-test.
/// One mint call with ~0.1 KB metadata (NFT #418): 7.0M cycles.
const MINT_CALL_CYCLES: u64 = 7_000_000;
/// Extra cost per uploaded byte: (83.2M - 7.0M) / 35.2 KB photo (NFT #520).
const MINT_CYCLES_PER_BYTE: u64 = 2_114;
/// One item inside a batch of 100 metadata records: 0.3M cycles.
const BATCH_ITEM_CYCLES: u64 = 300_000;
/// Storage fee of a 13-node subnet: 127K cycles per GiB per second.
const STORAGE_CYCLES_PER_GIB_SEC: u128 = 127_000;
const SEC_PER_YEAR: u128 = 365 * 24 * 3600;
/// Platform margin on top of the cost price, in basis points. 0 = cost price.
const MINT_MARKUP_BPS: u64 = 0;
/// Display only: 1T cycles = 1 XDR.
const XDR_USD: f64 = 1.36643;
/// Approx. metadata stored with every NFT.
const METADATA_BYTES: u64 = 100;

#[derive(CandidType, Deserialize, Clone)]
struct Listing {
    id: u64,
    nft_id: String,
    seller: Principal,
    price_e8s: u64,
    sold: bool,
}

#[derive(CandidType, Deserialize, Clone)]
struct Receipt {
    listing_id: u64,
    nft_id: String,
    buyer: Principal,
    seller: Principal,
    price_e8s: u64,
    seller_got_e8s: u64,
    platform_fee_e8s: u64,
    pull_block: Nat,
    payout_block: Nat,
    timestamp_ns: u64,
}

#[derive(CandidType)]
struct Config {
    ledger: Principal,
    ledger_fee_e8s: u64,
    platform_fee_bps: u16,
}

#[derive(CandidType, Deserialize, Clone, Copy, PartialEq)]
enum MintKind {
    Photo,
    Metadata,
    MetadataBatch,
}

#[derive(CandidType, Deserialize, Clone)]
struct MintQuoteRequest {
    kind: MintKind,
    /// Stored bytes per item (image + metadata).
    bytes: u64,
    count: u64,
    storage_years: u16,
}

#[derive(CandidType, Deserialize, Clone)]
struct MintQuote {
    kind: MintKind,
    bytes: u64,
    count: u64,
    storage_years: u16,
    mint_cycles_per_item: u64,
    storage_cycles_per_item_year: u64,
    total_cycles: Nat,
    markup_bps: u64,
    xdr_permyriad_per_icp: u64,
    rate_timestamp_seconds: u64,
    price_e8s: u64,
}

#[derive(CandidType)]
struct Pricing {
    mint_call_cycles: u64,
    mint_cycles_per_byte: u64,
    batch_item_cycles: u64,
    storage_cycles_per_gib_sec: u64,
    markup_bps: u64,
    xdr_usd: f64,
    xdr_permyriad_per_icp: u64,
    rate_timestamp_seconds: u64,
    ledger: Principal,
    ledger_fee_e8s: u64,
}

#[derive(CandidType, Deserialize)]
struct IcpXdrConversionRate {
    xdr_permyriad_per_icp: u64,
    timestamp_seconds: u64,
}

#[derive(CandidType, Deserialize)]
struct IcpXdrConversionRateResponse {
    data: IcpXdrConversionRate,
    hash_tree: serde_bytes::ByteBuf,
    certificate: serde_bytes::ByteBuf,
}

#[derive(Default, Clone, Copy)]
struct Rate {
    xdr_permyriad_per_icp: u64,
    timestamp_seconds: u64,
    fetched_ns: u64,
}

#[derive(Default)]
struct State {
    next_id: u64,
    listings: BTreeMap<u64, Listing>,
    receipts: Vec<Receipt>,
    rate: Rate,
    plans: Vec<Plan>,
    subscriptions: BTreeMap<Principal, Subscription>,
    purchases: Vec<PlanPurchase>,
    next_token: u64,
    next_image: u64,
    next_upload: u64,
    tokens: BTreeMap<u64, Token>,
    images: BTreeMap<u64, Vec<u8>>,
    uploads: BTreeMap<u64, Upload>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn ledger_id() -> Principal {
    Principal::from_text(ICP_LEDGER).unwrap()
}

fn account(owner: Principal) -> Account {
    Account { owner, subaccount: None }
}

fn set_sold(id: u64, sold: bool) {
    STATE.with_borrow_mut(|s| {
        if let Some(l) = s.listings.get_mut(&id) {
            l.sold = sold;
        }
    });
}

#[update]
fn create_listing(nft_id: String, price_e8s: u64) -> u64 {
    let seller = ic_cdk::api::msg_caller();
    if seller == Principal::anonymous() {
        ic_cdk::trap("anonymous caller cannot create listings");
    }
    // The seller payout must cover the ledger fee the canister pays.
    if price_e8s <= LEDGER_FEE_E8S * 2 {
        ic_cdk::trap("price too low");
    }
    STATE.with_borrow_mut(|s| {
        let id = s.next_id;
        s.next_id += 1;
        s.listings.insert(id, Listing { id, nft_id, seller, price_e8s, sold: false });
        id
    })
}

#[update]
async fn buy(listing_id: u64) -> Result<Receipt, String> {
    let buyer = ic_cdk::api::msg_caller();
    if buyer == Principal::anonymous() {
        return Err("anonymous caller cannot buy".into());
    }

    // Lock the listing before any await so a second `buy` cannot pay for it too.
    let listing = STATE.with_borrow_mut(|s| {
        let l = s.listings.get_mut(&listing_id).ok_or("listing not found")?;
        if l.sold {
            return Err("listing already sold");
        }
        if l.seller == buyer {
            return Err("seller cannot buy own listing");
        }
        l.sold = true;
        Ok(l.clone())
    })?;

    let now = ic_cdk::api::time();
    let me = ic_cdk::api::canister_self();

    // 1. Pull the full price from the buyer into this canister (escrow).
    let pull = TransferFromArgs {
        spender_subaccount: None,
        from: account(buyer),
        to: account(me),
        amount: Nat::from(listing.price_e8s),
        fee: Some(Nat::from(LEDGER_FEE_E8S)),
        memo: None,
        created_at_time: Some(now),
    };
    let pull_result = Call::unbounded_wait(ledger_id(), "icrc2_transfer_from")
        .with_arg(pull)
        .await
        .map_err(|e| format!("ledger call failed: {e:?}"))
        .and_then(|r| {
            r.candid::<Result<Nat, TransferFromError>>()
                .map_err(|e| format!("decode failed: {e:?}"))
        });
    let pull_block = match pull_result {
        Ok(Ok(block)) => block,
        Ok(Err(e)) => {
            set_sold(listing_id, false);
            return Err(match e {
                TransferFromError::InsufficientAllowance { allowance } => format!(
                    "insufficient allowance: {allowance} e8s, need {} e8s (price + fee)",
                    listing.price_e8s + LEDGER_FEE_E8S
                ),
                TransferFromError::InsufficientFunds { balance } => {
                    format!("insufficient funds: {balance} e8s")
                }
                other => format!("transfer_from error: {other:?}"),
            });
        }
        Err(e) => {
            set_sold(listing_id, false);
            return Err(e);
        }
    };

    // 2. Pay the seller: price minus platform fee minus the ledger fee for this transfer.
    let platform_fee = listing.price_e8s * PLATFORM_FEE_BPS as u64 / 10_000;
    let seller_got = listing.price_e8s - platform_fee - LEDGER_FEE_E8S;
    let payout = TransferArg {
        from_subaccount: None,
        to: account(listing.seller),
        amount: Nat::from(seller_got),
        fee: Some(Nat::from(LEDGER_FEE_E8S)),
        memo: None,
        created_at_time: Some(now),
    };
    let payout_result = Call::unbounded_wait(ledger_id(), "icrc1_transfer")
        .with_arg(payout)
        .await
        .map_err(|e| format!("ledger call failed: {e:?}"))
        .and_then(|r| {
            r.candid::<Result<Nat, TransferError>>()
                .map_err(|e| format!("decode failed: {e:?}"))
        });
    let payout_block = match payout_result {
        Ok(Ok(block)) => block,
        // Funds are already in escrow: keep the listing sold and report it.
        Ok(Err(e)) => {
            return Err(format!("paid (block {pull_block}) but seller payout failed: {e:?}"))
        }
        Err(e) => return Err(format!("paid (block {pull_block}) but seller payout failed: {e}")),
    };

    let receipt = Receipt {
        listing_id,
        nft_id: listing.nft_id,
        buyer,
        seller: listing.seller,
        price_e8s: listing.price_e8s,
        seller_got_e8s: seller_got,
        platform_fee_e8s: platform_fee,
        pull_block,
        payout_block,
        timestamp_ns: now,
    };
    STATE.with_borrow_mut(|s| s.receipts.push(receipt.clone()));
    Ok(receipt)
}

#[query]
fn get_listing(id: u64) -> Option<Listing> {
    STATE.with_borrow(|s| s.listings.get(&id).cloned())
}

#[query]
fn list_listings() -> Vec<Listing> {
    STATE.with_borrow(|s| s.listings.values().cloned().collect())
}

#[query]
fn receipts() -> Vec<Receipt> {
    STATE.with_borrow(|s| s.receipts.clone())
}

#[query]
fn config() -> Config {
    Config {
        ledger: ledger_id(),
        ledger_fee_e8s: LEDGER_FEE_E8S,
        platform_fee_bps: PLATFORM_FEE_BPS,
    }
}

/// ICP held by this canister (collected platform fees).
#[update]
async fn platform_balance() -> Nat {
    Call::unbounded_wait(ledger_id(), "icrc1_balance_of")
        .with_arg(account(ic_cdk::api::canister_self()))
        .await
        .expect("icrc1_balance_of failed")
        .candid::<Nat>()
        .expect("decode failed")
}

// ── Mint pricing ────────────────────────────────────────────────────────────

fn mint_cycles_per_item(kind: MintKind, bytes: u64) -> u64 {
    match kind {
        MintKind::Photo => MINT_CALL_CYCLES + bytes * MINT_CYCLES_PER_BYTE,
        MintKind::Metadata => MINT_CALL_CYCLES,
        MintKind::MetadataBatch => BATCH_ITEM_CYCLES,
    }
}

fn storage_cycles_per_item_year(bytes: u64) -> u64 {
    (bytes as u128 * STORAGE_CYCLES_PER_GIB_SEC * SEC_PER_YEAR / (1u128 << 30)) as u64
}

fn build_quote(req: &MintQuoteRequest, rate: Rate) -> Result<MintQuote, String> {
    if req.count == 0 || req.count > 1_000_000 {
        return Err("count must be 1..=1000000".into());
    }
    if req.bytes > 100 * 1024 * 1024 {
        return Err("bytes too large".into());
    }
    if rate.xdr_permyriad_per_icp == 0 {
        return Err("ICP/XDR rate not loaded, call refresh_rate".into());
    }
    let mint = mint_cycles_per_item(req.kind, req.bytes);
    let storage = storage_cycles_per_item_year(req.bytes);
    let per_item = mint as u128 + storage as u128 * req.storage_years as u128;
    let cost = per_item * req.count as u128;
    let total = cost + cost * MINT_MARKUP_BPS as u128 / 10_000;
    // 1T cycles = 1 XDR and 1 ICP = permyriad / 10_000 XDR, so e8s = cycles / permyriad.
    let p = rate.xdr_permyriad_per_icp as u128;
    let price_e8s = total.div_ceil(p).max(1) as u64;
    Ok(MintQuote {
        kind: req.kind,
        bytes: req.bytes,
        count: req.count,
        storage_years: req.storage_years,
        mint_cycles_per_item: mint,
        storage_cycles_per_item_year: storage,
        total_cycles: Nat::from(total),
        markup_bps: MINT_MARKUP_BPS,
        xdr_permyriad_per_icp: rate.xdr_permyriad_per_icp,
        rate_timestamp_seconds: rate.timestamp_seconds,
        price_e8s,
    })
}

async fn fresh_rate() -> Result<Rate, String> {
    let cached = STATE.with_borrow(|s| s.rate);
    let now = ic_cdk::api::time();
    if cached.xdr_permyriad_per_icp > 0 && now - cached.fetched_ns < RATE_MAX_AGE_NS {
        return Ok(cached);
    }
    let res = Call::unbounded_wait(Principal::from_text(CMC).unwrap(), "get_icp_xdr_conversion_rate")
        .await
        .map_err(|e| format!("CMC call failed: {e:?}"))?
        .candid::<IcpXdrConversionRateResponse>()
        .map_err(|e| format!("CMC decode failed: {e:?}"))?;
    let rate = Rate {
        xdr_permyriad_per_icp: res.data.xdr_permyriad_per_icp,
        timestamp_seconds: res.data.timestamp_seconds,
        fetched_ns: now,
    };
    STATE.with_borrow_mut(|s| s.rate = rate);
    Ok(rate)
}

/// Loads the ICP/XDR rate from the CMC (cached for 10 minutes).
#[update]
async fn refresh_rate() -> Result<u64, String> {
    fresh_rate().await.map(|r| r.xdr_permyriad_per_icp)
}

/// Prices a mint job with the cached rate. Call `refresh_rate` first.
#[query]
fn quote_mint(req: MintQuoteRequest) -> Result<MintQuote, String> {
    build_quote(&req, STATE.with_borrow(|s| s.rate))
}

// ── Plans ───────────────────────────────────────────────────────────────────

/// Price of one USD in XDR micro-units: 1 XDR = $1.36643. Display and plan conversion only.
const XDR_USD_MICROS: u128 = 1_366_430;
const NS_PER_DAY: u64 = 24 * 3600 * 1_000_000_000;

/// A package a company buys: N mints of each kind for `period_days`.
#[derive(CandidType, Deserialize, Clone)]
struct Plan {
    id: String,
    name: String,
    price_usd_cents: u64,
    period_days: u32,
    photo_mints: u64,
    metadata_mints: u64,
    max_photo_kb: u64,
    storage_years: u16,
}

#[derive(CandidType, Deserialize, Clone)]
struct PlanQuote {
    plan: Plan,
    price_e8s: u64,
    xdr_permyriad_per_icp: u64,
    rate_timestamp_seconds: u64,
    /// Worst-case cycles to serve the plan: every photo at `max_photo_kb`.
    max_cost_cycles: Nat,
}

#[derive(CandidType, Deserialize, Clone)]
struct Subscription {
    owner: Principal,
    plan_id: String,
    photo_mints_left: u64,
    metadata_mints_left: u64,
    max_photo_kb: u64,
    expires_at_ns: u64,
}

#[derive(CandidType, Deserialize, Clone)]
struct PlanPurchase {
    id: u64,
    buyer: Principal,
    plan_id: String,
    price_e8s: u64,
    block: Nat,
    timestamp_ns: u64,
}

#[derive(CandidType, Deserialize, Clone, Copy)]
enum UsageKind {
    Photo,
    Metadata,
}

fn default_plans() -> Vec<Plan> {
    let plan = |id: &str, name: &str, cents, photo, meta, kb| Plan {
        id: id.into(),
        name: name.into(),
        price_usd_cents: cents,
        period_days: 30,
        photo_mints: photo,
        metadata_mints: meta,
        max_photo_kb: kb,
        storage_years: 1,
    };
    vec![
        plan("start", "Старт", 900, 100, 1_000, 1_024),
        plan("business", "Бізнес", 4_900, 1_000, 10_000, 2_048),
        plan("pro", "Про", 19_900, 10_000, 100_000, 2_048),
    ]
}

fn find_plan(id: &str) -> Result<Plan, String> {
    STATE.with_borrow(|s| s.plans.iter().find(|p| p.id == id).cloned())
        .ok_or_else(|| format!("plan {id} not found"))
}

fn plan_max_cost_cycles(plan: &Plan) -> u128 {
    let years = plan.storage_years as u128;
    let photo_bytes = plan.max_photo_kb * 1024 + METADATA_BYTES;
    let photo = mint_cycles_per_item(MintKind::Photo, photo_bytes) as u128
        + storage_cycles_per_item_year(photo_bytes) as u128 * years;
    let meta = mint_cycles_per_item(MintKind::MetadataBatch, METADATA_BYTES) as u128
        + storage_cycles_per_item_year(METADATA_BYTES) as u128 * years;
    photo * plan.photo_mints as u128 + meta * plan.metadata_mints as u128
}

fn build_plan_quote(plan: Plan, rate: Rate) -> Result<PlanQuote, String> {
    if rate.xdr_permyriad_per_icp == 0 {
        return Err("ICP/XDR rate not loaded, call refresh_rate".into());
    }
    // e8s = usd / XDR_USD * 10_000 / permyriad * 1e8
    //     = cents * 1e16 / (XDR_USD_MICROS * permyriad)
    let price = (plan.price_usd_cents as u128 * 10u128.pow(16))
        .div_ceil(XDR_USD_MICROS * rate.xdr_permyriad_per_icp as u128);
    Ok(PlanQuote {
        max_cost_cycles: Nat::from(plan_max_cost_cycles(&plan)),
        plan,
        price_e8s: price as u64,
        xdr_permyriad_per_icp: rate.xdr_permyriad_per_icp,
        rate_timestamp_seconds: rate.timestamp_seconds,
    })
}

fn require_controller() -> Result<(), String> {
    if ic_cdk::api::is_controller(&ic_cdk::api::msg_caller()) {
        Ok(())
    } else {
        Err("only a controller can call this".into())
    }
}

/// All plans priced in ICP with the cached rate. Call `refresh_rate` first.
#[query]
fn plans() -> Result<Vec<PlanQuote>, String> {
    let (plans, rate) = STATE.with_borrow(|s| (s.plans.clone(), s.rate));
    plans.into_iter().map(|p| build_plan_quote(p, rate)).collect()
}

/// Replaces the plan list. Controller only (the MarkICP backend or admin).
#[update]
fn set_plans(plans: Vec<Plan>) -> Result<(), String> {
    require_controller()?;
    STATE.with_borrow_mut(|s| s.plans = plans);
    Ok(())
}

/// Buys a plan. The caller must first approve this canister on the ledger for
/// at least `price_e8s + ledger fee`. `max_price_e8s` guards against a rate change
/// between the quote and the payment. Credits add up; the period extends from the
/// later of now and the current expiry.
#[update]
async fn buy_plan(plan_id: String, max_price_e8s: u64) -> Result<Subscription, String> {
    let buyer = ic_cdk::api::msg_caller();
    if buyer == Principal::anonymous() {
        return Err("anonymous caller cannot pay".into());
    }
    let quote = build_plan_quote(find_plan(&plan_id)?, fresh_rate().await?)?;
    if quote.price_e8s > max_price_e8s {
        return Err(format!(
            "price changed: {} e8s, max allowed {max_price_e8s} e8s",
            quote.price_e8s
        ));
    }

    let now = ic_cdk::api::time();
    let args = TransferFromArgs {
        spender_subaccount: None,
        from: account(buyer),
        to: account(ic_cdk::api::canister_self()),
        amount: Nat::from(quote.price_e8s),
        fee: Some(Nat::from(LEDGER_FEE_E8S)),
        memo: None,
        created_at_time: Some(now),
    };
    let block = Call::unbounded_wait(ledger_id(), "icrc2_transfer_from")
        .with_arg(args)
        .await
        .map_err(|e| format!("ledger call failed: {e:?}"))?
        .candid::<Result<Nat, TransferFromError>>()
        .map_err(|e| format!("decode failed: {e:?}"))?
        .map_err(|e| match e {
            TransferFromError::InsufficientAllowance { allowance } => format!(
                "insufficient allowance: {allowance} e8s, need {} e8s (price + fee)",
                quote.price_e8s + LEDGER_FEE_E8S
            ),
            TransferFromError::InsufficientFunds { balance } => {
                format!("insufficient funds: {balance} e8s")
            }
            other => format!("transfer_from error: {other:?}"),
        })?;

    let plan = quote.plan;
    let sub = STATE.with_borrow_mut(|s| {
        s.purchases.push(PlanPurchase {
            id: s.purchases.len() as u64,
            buyer,
            plan_id: plan.id.clone(),
            price_e8s: quote.price_e8s,
            block,
            timestamp_ns: now,
        });
        let period = plan.period_days as u64 * NS_PER_DAY;
        let sub = s.subscriptions.entry(buyer).or_insert(Subscription {
            owner: buyer,
            plan_id: plan.id.clone(),
            photo_mints_left: 0,
            metadata_mints_left: 0,
            max_photo_kb: 0,
            expires_at_ns: now,
        });
        if sub.expires_at_ns <= now {
            // Expired credits do not carry over.
            sub.photo_mints_left = 0;
            sub.metadata_mints_left = 0;
            sub.expires_at_ns = now;
        }
        sub.plan_id = plan.id.clone();
        sub.photo_mints_left += plan.photo_mints;
        sub.metadata_mints_left += plan.metadata_mints;
        sub.max_photo_kb = sub.max_photo_kb.max(plan.max_photo_kb);
        sub.expires_at_ns += period;
        sub.clone()
    });
    Ok(sub)
}

#[query]
fn subscription_of(owner: Principal) -> Option<Subscription> {
    STATE.with_borrow(|s| s.subscriptions.get(&owner).cloned())
}

#[query]
fn purchases_of(owner: Principal) -> Vec<PlanPurchase> {
    STATE.with_borrow(|s| s.purchases.iter().filter(|p| p.buyer == owner).cloned().collect())
}

/// Checks the plan of `owner` and takes `count` credits of `kind`.
/// `bytes` is the photo size, checked against the plan limit.
fn spend_credits(s: &mut State, owner: Principal, kind: UsageKind, count: u64, bytes: u64) -> Result<Subscription, String> {
    let now = ic_cdk::api::time();
    let sub = s.subscriptions.get_mut(&owner).ok_or("no plan: buy a plan first")?;
    if sub.expires_at_ns <= now {
        return Err("plan expired".into());
    }
    let left = match kind {
        UsageKind::Photo => {
            if bytes > sub.max_photo_kb * 1024 {
                return Err(format!("photo is larger than {} KB allowed by the plan", sub.max_photo_kb));
            }
            &mut sub.photo_mints_left
        }
        UsageKind::Metadata => &mut sub.metadata_mints_left,
    };
    if *left < count {
        return Err(format!("not enough mints: {} left, need {count}", *left));
    }
    *left -= count;
    Ok(sub.clone())
}

/// Spends plan credits for a mint done elsewhere. Controller only (MarkICP backend).
#[update]
fn consume_mints(owner: Principal, kind: UsageKind, count: u64, bytes: u64) -> Result<Subscription, String> {
    require_controller()?;
    STATE.with_borrow_mut(|s| spend_credits(s, owner, kind, count, bytes))
}

// ── NFTs (ICRC-7 subset) ────────────────────────────────────────────────────
//
// Photo mint: `create_upload` → `append_chunk` × N (≤ 1.9 MB each, ingress limit)
// → `mint_uploaded(upload_id, metadata, copies)`. `copies > 1` makes editions that
// share one stored image. Credits are taken inside the mint call, so a token
// exists only if the plan paid for it. The canister pays the cycles (reverse gas).

const MAX_METADATA_BYTES: usize = 8 * 1024;
const MAX_COPIES: u32 = 1_000;

#[derive(CandidType, Deserialize, Clone)]
struct Token {
    owner: Principal,
    metadata: String,
    image_id: Option<u64>,
    minted_at_ns: u64,
}

struct Upload {
    owner: Principal,
    bytes: Vec<u8>,
}

#[derive(CandidType)]
struct MintResult {
    token_ids: Vec<u64>,
    image_bytes: u64,
    subscription: Subscription,
}

#[derive(CandidType)]
struct NftStats {
    cycles: Nat,
    tokens: u64,
    images: u64,
    image_bytes: u64,
    heap_bytes: u64,
}

/// ICRC-3 value, the subset this canister returns.
#[derive(CandidType)]
enum Value {
    Nat(Nat),
    Text(String),
}

fn caller_not_anonymous() -> Result<Principal, String> {
    let caller = ic_cdk::api::msg_caller();
    if caller == Principal::anonymous() {
        return Err("connect a wallet: anonymous caller".into());
    }
    Ok(caller)
}

fn check_metadata(metadata: &str) -> Result<(), String> {
    if metadata.len() > MAX_METADATA_BYTES {
        return Err(format!("metadata over {MAX_METADATA_BYTES} bytes"));
    }
    Ok(())
}

#[update]
fn create_upload() -> Result<u64, String> {
    let owner = caller_not_anonymous()?;
    let now = ic_cdk::api::time();
    STATE.with_borrow_mut(|s| {
        // Uploads cost cycles too: only callers with photo credits may start one.
        match s.subscriptions.get(&owner) {
            Some(sub) if sub.expires_at_ns > now && sub.photo_mints_left > 0 => {}
            _ => return Err("no active plan with photo mints: buy a plan first".into()),
        }
        let id = s.next_upload;
        s.next_upload += 1;
        s.uploads.insert(id, Upload { owner, bytes: Vec::new() });
        Ok(id)
    })
}

/// Appends a chunk; returns the upload size so far.
#[update]
fn append_chunk(upload_id: u64, chunk: serde_bytes::ByteBuf) -> Result<u64, String> {
    let caller = caller_not_anonymous()?;
    STATE.with_borrow_mut(|s| {
        let limit = s.subscriptions.get(&caller).map(|sub| sub.max_photo_kb * 1024).unwrap_or(0);
        let up = s.uploads.get_mut(&upload_id).ok_or("upload not found")?;
        if up.owner != caller {
            return Err("not your upload".into());
        }
        if (up.bytes.len() + chunk.len()) as u64 > limit {
            return Err(format!("photo is larger than {} KB allowed by the plan", limit / 1024));
        }
        up.bytes.extend_from_slice(&chunk);
        Ok(up.bytes.len() as u64)
    })
}

#[update]
fn mint_uploaded(upload_id: u64, metadata: String, copies: u32) -> Result<MintResult, String> {
    let owner = caller_not_anonymous()?;
    check_metadata(&metadata)?;
    if copies == 0 || copies > MAX_COPIES {
        return Err(format!("copies must be 1..={MAX_COPIES}"));
    }
    let now = ic_cdk::api::time();
    STATE.with_borrow_mut(|s| {
        let up = s.uploads.get(&upload_id).ok_or("upload not found")?;
        if up.owner != owner {
            return Err("not your upload".into());
        }
        if up.bytes.is_empty() {
            return Err("upload is empty".into());
        }
        let size = up.bytes.len() as u64;
        let subscription = spend_credits(s, owner, UsageKind::Photo, copies as u64, size)?;

        let bytes = s.uploads.remove(&upload_id).unwrap().bytes;
        let image_id = s.next_image;
        s.next_image += 1;
        s.images.insert(image_id, bytes);

        let token_ids = (0..copies)
            .map(|_| {
                let id = s.next_token;
                s.next_token += 1;
                s.tokens.insert(id, Token { owner, metadata: metadata.clone(), image_id: Some(image_id), minted_at_ns: now });
                id
            })
            .collect();
        Ok(MintResult { token_ids, image_bytes: size, subscription })
    })
}

/// Mints metadata-only NFTs, one per entry.
#[update]
fn mint_metadata(items: Vec<String>) -> Result<MintResult, String> {
    let owner = caller_not_anonymous()?;
    if items.is_empty() || items.len() > MAX_COPIES as usize {
        return Err(format!("items must be 1..={MAX_COPIES}"));
    }
    for m in &items {
        check_metadata(m)?;
    }
    let now = ic_cdk::api::time();
    STATE.with_borrow_mut(|s| {
        let subscription = spend_credits(s, owner, UsageKind::Metadata, items.len() as u64, 0)?;
        let token_ids = items
            .into_iter()
            .map(|metadata| {
                let id = s.next_token;
                s.next_token += 1;
                s.tokens.insert(id, Token { owner, metadata, image_id: None, minted_at_ns: now });
                id
            })
            .collect();
        Ok(MintResult { token_ids, image_bytes: 0, subscription })
    })
}

#[query]
fn icrc7_name() -> String {
    "Marki".into()
}

#[query]
fn icrc7_symbol() -> String {
    "MARKI".into()
}

#[query]
fn icrc7_total_supply() -> Nat {
    Nat::from(STATE.with_borrow(|s| s.tokens.len()))
}

fn nat_to_u64(n: &Nat) -> Option<u64> {
    n.0.to_string().parse().ok()
}

#[query]
fn icrc7_owner_of(ids: Vec<Nat>) -> Vec<Option<Account>> {
    STATE.with_borrow(|s| {
        ids.iter()
            .map(|id| nat_to_u64(id).and_then(|id| s.tokens.get(&id)).map(|t| account(t.owner)))
            .collect()
    })
}

#[query]
fn icrc7_tokens_of(acc: Account, prev: Option<Nat>, take: Option<Nat>) -> Vec<Nat> {
    let start = prev.as_ref().and_then(nat_to_u64).map(|p| p + 1).unwrap_or(0);
    let take = take.as_ref().and_then(nat_to_u64).unwrap_or(100).min(1000) as usize;
    STATE.with_borrow(|s| {
        s.tokens
            .range(start..)
            .filter(|(_, t)| acc.subaccount.is_none() && t.owner == acc.owner)
            .take(take)
            .map(|(id, _)| Nat::from(*id))
            .collect()
    })
}

#[query]
fn icrc7_token_metadata(ids: Vec<Nat>) -> Vec<Option<Vec<(String, Value)>>> {
    STATE.with_borrow(|s| {
        ids.iter()
            .map(|id| {
                let t = s.tokens.get(&nat_to_u64(id)?)?;
                let mut fields = vec![
                    ("marki:metadata".to_string(), Value::Text(t.metadata.clone())),
                    ("marki:minted_at_ns".to_string(), Value::Nat(Nat::from(t.minted_at_ns))),
                ];
                if let Some(img) = t.image_id.and_then(|i| s.images.get(&i)) {
                    fields.push(("marki:image_bytes".to_string(), Value::Nat(Nat::from(img.len()))));
                }
                Some(fields)
            })
            .collect()
    })
}

#[query]
fn get_token(id: u64) -> Option<Token> {
    STATE.with_borrow(|s| s.tokens.get(&id).cloned())
}

#[query]
fn get_image(token_id: u64) -> Option<serde_bytes::ByteBuf> {
    STATE.with_borrow(|s| {
        let image_id = s.tokens.get(&token_id)?.image_id?;
        s.images.get(&image_id).map(|b| serde_bytes::ByteBuf::from(b.clone()))
    })
}

/// Cycle balance and storage. Read before and after a mint to see its real cost.
#[query]
fn nft_stats() -> NftStats {
    STATE.with_borrow(|s| NftStats {
        cycles: Nat::from(ic_cdk::api::canister_cycle_balance()),
        tokens: s.tokens.len() as u64,
        images: s.images.len() as u64,
        image_bytes: s.images.values().map(|b| b.len() as u64).sum(),
        heap_bytes: (core::arch::wasm32::memory_size(0) * 65_536) as u64,
    })
}

#[ic_cdk::init]
fn init() {
    STATE.with_borrow_mut(|s| s.plans = default_plans());
}

#[ic_cdk::post_upgrade]
fn post_upgrade() {
    init();
}

#[query]
fn pricing() -> Pricing {
    let rate = STATE.with_borrow(|s| s.rate);
    Pricing {
        mint_call_cycles: MINT_CALL_CYCLES,
        mint_cycles_per_byte: MINT_CYCLES_PER_BYTE,
        batch_item_cycles: BATCH_ITEM_CYCLES,
        storage_cycles_per_gib_sec: STORAGE_CYCLES_PER_GIB_SEC as u64,
        markup_bps: MINT_MARKUP_BPS,
        xdr_usd: XDR_USD,
        xdr_permyriad_per_icp: rate.xdr_permyriad_per_icp,
        rate_timestamp_seconds: rate.timestamp_seconds,
        ledger: ledger_id(),
        ledger_fee_e8s: LEDGER_FEE_E8S,
    }
}

ic_cdk::export_candid!();
