//! MarkICP payments canister.
//!
//! Marketplace (ICRC-2): a seller lists an NFT with `create_listing`, the buyer
//! approves this canister on the ledger and calls `buy`; the canister pulls the
//! price into escrow, keeps the platform fee and pays the seller.
//!
//! Plans: a company or user buys a plan (N photo mints + M metadata mints for 30
//! days) with `icrc2_approve` + `buy_plan`. Prices are in USD and converted to the
//! ledger token with the ICP/XDR rate of the cycles minting canister (CMC).
//!
//! NFTs: ICRC-7 subset. `mint_uploaded` / `mint_metadata` take plan credits and
//! mint in the same call; images are stored in the canister.
//!
//! Storage: everything that must survive upgrades lives in stable memory
//! (`ic-stable-structures`). Only in-progress photo uploads and the cached
//! ICP/XDR rate are heap-only; an upgrade drops them and nothing else.
//!
//! The ledger is chosen at install time (`InitArg.ledger`, default ICP) so the
//! same code runs on TESTICP for mainnet tests and on ICP for production.

use candid::{CandidType, Nat, Principal};
use ic_cdk::call::Call;
use ic_cdk::{query, update};
use ic_stable_structures::memory_manager::{MemoryId, MemoryManager, VirtualMemory};
use ic_stable_structures::storable::{Bound, Storable};
use ic_stable_structures::{DefaultMemoryImpl, StableBTreeMap, StableCell};
use icrc_ledger_types::icrc1::account::Account;
use icrc_ledger_types::icrc1::transfer::{TransferArg, TransferError};
use icrc_ledger_types::icrc2::transfer_from::{TransferFromArgs, TransferFromError};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;

const ICP_LEDGER: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";
const DEFAULT_LEDGER_FEE_E8S: u64 = 10_000;
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

// ── Stable storage ──────────────────────────────────────────────────────────

type Memory = VirtualMemory<DefaultMemoryImpl>;

/// CBOR-encoded, unbounded: new optional fields can be added later without
/// breaking stored records.
macro_rules! cbor_storable {
    ($($t:ty),* $(,)?) => {$(
        impl Storable for $t {
            const BOUND: Bound = Bound::Unbounded;
            fn to_bytes(&self) -> Cow<'_, [u8]> {
                let mut buf = vec![];
                ciborium::into_writer(self, &mut buf).expect("encode");
                Cow::Owned(buf)
            }
            fn into_bytes(self) -> Vec<u8> {
                let mut buf = vec![];
                ciborium::into_writer(&self, &mut buf).expect("encode");
                buf
            }
            fn from_bytes(bytes: Cow<'_, [u8]>) -> Self {
                ciborium::from_reader(bytes.as_ref()).expect("decode")
            }
        }
    )*};
}

/// Small singleton state: ledger, fee and id counters.
#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    ledger: Principal,
    ledger_fee_e8s: u64,
    next_listing: u64,
    next_receipt: u64,
    next_purchase: u64,
    next_token: u64,
    next_image: u64,
    image_bytes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ledger: Principal::from_text(ICP_LEDGER).unwrap(),
            ledger_fee_e8s: DEFAULT_LEDGER_FEE_E8S,
            next_listing: 0,
            next_receipt: 0,
            next_purchase: 0,
            next_token: 0,
            next_image: 0,
            image_bytes: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct PlanList(Vec<Plan>);

#[derive(Serialize, Deserialize, Clone)]
struct Image(#[serde(with = "serde_bytes")] Vec<u8>);

cbor_storable!(Settings, PlanList, Image, Listing, Receipt, Subscription, PlanPurchase, Token);

// Each structure owns one MemoryId. Never reuse or renumber them.
const MEM_SETTINGS: MemoryId = MemoryId::new(0);
const MEM_PLANS: MemoryId = MemoryId::new(1);
const MEM_LISTINGS: MemoryId = MemoryId::new(2);
const MEM_RECEIPTS: MemoryId = MemoryId::new(3);
const MEM_SUBSCRIPTIONS: MemoryId = MemoryId::new(4);
const MEM_PURCHASES: MemoryId = MemoryId::new(5);
const MEM_TOKENS: MemoryId = MemoryId::new(6);
const MEM_IMAGES: MemoryId = MemoryId::new(7);
const MEM_OWNER_TOKENS: MemoryId = MemoryId::new(8);

thread_local! {
    static MM: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    static SETTINGS: RefCell<StableCell<Settings, Memory>> =
        RefCell::new(StableCell::init(mem(MEM_SETTINGS), Settings::default()));
    static PLANS: RefCell<StableCell<PlanList, Memory>> =
        RefCell::new(StableCell::init(mem(MEM_PLANS), PlanList::default()));
    static LISTINGS: RefCell<StableBTreeMap<u64, Listing, Memory>> =
        RefCell::new(StableBTreeMap::init(mem(MEM_LISTINGS)));
    static RECEIPTS: RefCell<StableBTreeMap<u64, Receipt, Memory>> =
        RefCell::new(StableBTreeMap::init(mem(MEM_RECEIPTS)));
    /// principal text -> subscription
    static SUBSCRIPTIONS: RefCell<StableBTreeMap<String, Subscription, Memory>> =
        RefCell::new(StableBTreeMap::init(mem(MEM_SUBSCRIPTIONS)));
    static PURCHASES: RefCell<StableBTreeMap<u64, PlanPurchase, Memory>> =
        RefCell::new(StableBTreeMap::init(mem(MEM_PURCHASES)));
    static TOKENS: RefCell<StableBTreeMap<u64, Token, Memory>> =
        RefCell::new(StableBTreeMap::init(mem(MEM_TOKENS)));
    static IMAGES: RefCell<StableBTreeMap<u64, Image, Memory>> =
        RefCell::new(StableBTreeMap::init(mem(MEM_IMAGES)));
    /// "<owner principal>#<token id, 20 digits>" -> () : tokens of an owner, in id order.
    static OWNER_TOKENS: RefCell<StableBTreeMap<String, (), Memory>> =
        RefCell::new(StableBTreeMap::init(mem(MEM_OWNER_TOKENS)));

    // Heap only: cheap to lose on upgrade.
    static RATE: RefCell<Rate> = RefCell::new(Rate::default());
    static UPLOADS: RefCell<BTreeMap<u64, Upload>> = RefCell::new(BTreeMap::new());
    static NEXT_UPLOAD: RefCell<u64> = const { RefCell::new(0) };
}

fn mem(id: MemoryId) -> Memory {
    MM.with(|m| m.borrow().get(id))
}

fn settings() -> Settings {
    SETTINGS.with(|s| s.borrow().get().clone())
}

fn update_settings<R>(f: impl FnOnce(&mut Settings) -> R) -> R {
    SETTINGS.with(|cell| {
        let mut cell = cell.borrow_mut();
        let mut s = cell.get().clone();
        let r = f(&mut s);
        cell.set(s);
        r
    })
}

fn owner_key(owner: &Principal, token: u64) -> String {
    format!("{}#{:020}", owner.to_text(), token)
}

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(CandidType, Serialize, Deserialize, Clone)]
struct Listing {
    id: u64,
    nft_id: String,
    seller: Principal,
    price_e8s: u64,
    sold: bool,
}

#[derive(CandidType, Serialize, Deserialize, Clone)]
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

#[derive(CandidType, Deserialize, Default)]
struct InitArg {
    /// Ledger for all payments. Default: ICP. TESTICP for mainnet tests.
    ledger: Option<Principal>,
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

fn ledger_id() -> Principal {
    settings().ledger
}

fn ledger_fee() -> u64 {
    settings().ledger_fee_e8s
}

fn account(owner: Principal) -> Account {
    Account { owner, subaccount: None }
}

fn require_controller() -> Result<(), String> {
    if ic_cdk::api::is_controller(&ic_cdk::api::msg_caller()) {
        Ok(())
    } else {
        Err("only a controller can call this".into())
    }
}

fn caller_not_anonymous() -> Result<Principal, String> {
    let caller = ic_cdk::api::msg_caller();
    if caller == Principal::anonymous() {
        return Err("connect a wallet: anonymous caller".into());
    }
    Ok(caller)
}

// ── Marketplace ─────────────────────────────────────────────────────────────

fn set_sold(id: u64, sold: bool) {
    LISTINGS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(mut l) = m.get(&id) {
            l.sold = sold;
            m.insert(id, l);
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
    if price_e8s <= ledger_fee() * 2 {
        ic_cdk::trap("price too low");
    }
    let id = update_settings(|s| {
        let id = s.next_listing;
        s.next_listing += 1;
        id
    });
    LISTINGS.with(|m| m.borrow_mut().insert(id, Listing { id, nft_id, seller, price_e8s, sold: false }));
    id
}

#[update]
async fn buy(listing_id: u64) -> Result<Receipt, String> {
    let buyer = ic_cdk::api::msg_caller();
    if buyer == Principal::anonymous() {
        return Err("anonymous caller cannot buy".into());
    }

    // Lock the listing before any await so a second `buy` cannot pay for it too.
    let listing = LISTINGS.with(|m| {
        let mut m = m.borrow_mut();
        let mut l = m.get(&listing_id).ok_or("listing not found")?;
        if l.sold {
            return Err("listing already sold");
        }
        if l.seller == buyer {
            return Err("seller cannot buy own listing");
        }
        l.sold = true;
        m.insert(listing_id, l.clone());
        Ok(l)
    })?;

    let now = ic_cdk::api::time();
    let me = ic_cdk::api::canister_self();
    let fee = ledger_fee();

    // 1. Pull the full price from the buyer into this canister (escrow).
    let pull = TransferFromArgs {
        spender_subaccount: None,
        from: account(buyer),
        to: account(me),
        amount: Nat::from(listing.price_e8s),
        fee: Some(Nat::from(fee)),
        memo: None,
        created_at_time: Some(now),
    };
    let pull_result = Call::unbounded_wait(ledger_id(), "icrc2_transfer_from")
        .with_arg(pull)
        .await
        .map_err(|e| format!("ledger call failed: {e:?}"))
        .and_then(|r| r.candid::<Result<Nat, TransferFromError>>().map_err(|e| format!("decode failed: {e:?}")));
    let pull_block = match pull_result {
        Ok(Ok(block)) => block,
        Ok(Err(e)) => {
            set_sold(listing_id, false);
            return Err(match e {
                TransferFromError::InsufficientAllowance { allowance } => format!(
                    "insufficient allowance: {allowance} e8s, need {} e8s (price + fee)",
                    listing.price_e8s + fee
                ),
                TransferFromError::InsufficientFunds { balance } => format!("insufficient funds: {balance} e8s"),
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
    let seller_got = listing.price_e8s - platform_fee - fee;
    let payout = TransferArg {
        from_subaccount: None,
        to: account(listing.seller),
        amount: Nat::from(seller_got),
        fee: Some(Nat::from(fee)),
        memo: None,
        created_at_time: Some(now),
    };
    let payout_result = Call::unbounded_wait(ledger_id(), "icrc1_transfer")
        .with_arg(payout)
        .await
        .map_err(|e| format!("ledger call failed: {e:?}"))
        .and_then(|r| r.candid::<Result<Nat, TransferError>>().map_err(|e| format!("decode failed: {e:?}")));
    let payout_block = match payout_result {
        Ok(Ok(block)) => block,
        // Funds are already in escrow: keep the listing sold and report it.
        Ok(Err(e)) => return Err(format!("paid (block {pull_block}) but seller payout failed: {e:?}")),
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
    let id = update_settings(|s| {
        let id = s.next_receipt;
        s.next_receipt += 1;
        id
    });
    RECEIPTS.with(|m| m.borrow_mut().insert(id, receipt.clone()));
    Ok(receipt)
}

#[query]
fn get_listing(id: u64) -> Option<Listing> {
    LISTINGS.with(|m| m.borrow().get(&id))
}

#[query]
fn list_listings() -> Vec<Listing> {
    LISTINGS.with(|m| m.borrow().iter().map(|e| e.value()).collect())
}

#[query]
fn receipts() -> Vec<Receipt> {
    RECEIPTS.with(|m| m.borrow().iter().map(|e| e.value()).collect())
}

#[query]
fn config() -> Config {
    let s = settings();
    Config { ledger: s.ledger, ledger_fee_e8s: s.ledger_fee_e8s, platform_fee_bps: PLATFORM_FEE_BPS }
}

async fn ledger_balance() -> Result<Nat, String> {
    Call::unbounded_wait(ledger_id(), "icrc1_balance_of")
        .with_arg(account(ic_cdk::api::canister_self()))
        .await
        .map_err(|e| format!("icrc1_balance_of failed: {e:?}"))?
        .candid::<Nat>()
        .map_err(|e| format!("decode failed: {e:?}"))
}

/// Tokens held by this canister (paid plans and platform fees).
#[update]
async fn platform_balance() -> Nat {
    ledger_balance().await.unwrap_or_default()
}

/// Sends tokens held by this canister to `to`. Controller only.
/// `amount_e8s` is what `to` receives; the ledger fee is paid on top.
#[update]
async fn withdraw(to: Account, amount_e8s: u64) -> Result<Nat, String> {
    require_controller()?;
    if amount_e8s == 0 {
        return Err("amount must be positive".into());
    }
    let fee = ledger_fee();
    let args = TransferArg {
        from_subaccount: None,
        to,
        amount: Nat::from(amount_e8s),
        fee: Some(Nat::from(fee)),
        memo: None,
        created_at_time: Some(ic_cdk::api::time()),
    };
    Call::unbounded_wait(ledger_id(), "icrc1_transfer")
        .with_arg(args)
        .await
        .map_err(|e| format!("ledger call failed: {e:?}"))?
        .candid::<Result<Nat, TransferError>>()
        .map_err(|e| format!("decode failed: {e:?}"))?
        .map_err(|e| match e {
            TransferError::InsufficientFunds { balance } => format!("insufficient funds: {balance} e8s (fee {fee} e8s is extra)"),
            TransferError::BadFee { expected_fee } => format!("ledger fee changed to {expected_fee}; call refresh_rate and retry"),
            other => format!("transfer error: {other:?}"),
        })
}

/// Switches the payment ledger (e.g. TESTICP -> ICP) and reloads its fee. Controller only.
#[update]
async fn set_ledger(ledger: Principal) -> Result<u64, String> {
    require_controller()?;
    update_settings(|s| s.ledger = ledger);
    refresh_fee().await
}

async fn refresh_fee() -> Result<u64, String> {
    let fee = Call::unbounded_wait(ledger_id(), "icrc1_fee")
        .await
        .map_err(|e| format!("icrc1_fee failed: {e:?}"))?
        .candid::<Nat>()
        .map_err(|e| format!("decode failed: {e:?}"))?;
    let fee: u64 = fee.0.to_string().parse().map_err(|_| "fee too large".to_string())?;
    update_settings(|s| s.ledger_fee_e8s = fee);
    Ok(fee)
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
    let cached = RATE.with(|r| *r.borrow());
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
    RATE.with(|r| *r.borrow_mut() = rate);
    // Keep the ledger fee current too; a failure here must not block pricing.
    let _ = refresh_fee().await;
    Ok(rate)
}

/// Loads the ICP/XDR rate from the CMC (cached for 10 minutes) and the ledger fee.
#[update]
async fn refresh_rate() -> Result<u64, String> {
    fresh_rate().await.map(|r| r.xdr_permyriad_per_icp)
}

/// Prices a mint job with the cached rate. Call `refresh_rate` first.
#[query]
fn quote_mint(req: MintQuoteRequest) -> Result<MintQuote, String> {
    build_quote(&req, RATE.with(|r| *r.borrow()))
}

// ── Plans ───────────────────────────────────────────────────────────────────

/// Price of one USD in XDR micro-units: 1 XDR = $1.36643. Display and plan conversion only.
const XDR_USD_MICROS: u128 = 1_366_430;
const NS_PER_DAY: u64 = 24 * 3600 * 1_000_000_000;

/// A package a company buys: N mints of each kind for `period_days`.
#[derive(CandidType, Serialize, Deserialize, Clone)]
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

#[derive(CandidType, Serialize, Deserialize, Clone)]
struct Subscription {
    owner: Principal,
    plan_id: String,
    photo_mints_left: u64,
    metadata_mints_left: u64,
    max_photo_kb: u64,
    expires_at_ns: u64,
}

#[derive(CandidType, Serialize, Deserialize, Clone)]
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
    PLANS.with(|p| p.borrow().get().0.iter().find(|p| p.id == id).cloned())
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

/// All plans priced in ICP with the cached rate. Call `refresh_rate` first.
#[query]
fn plans() -> Result<Vec<PlanQuote>, String> {
    let plans = PLANS.with(|p| p.borrow().get().0.clone());
    let rate = RATE.with(|r| *r.borrow());
    plans.into_iter().map(|p| build_plan_quote(p, rate)).collect()
}

/// Replaces the plan list. Controller only (the MarkICP backend or admin).
#[update]
fn set_plans(plans: Vec<Plan>) -> Result<(), String> {
    require_controller()?;
    PLANS.with(|p| {
        p.borrow_mut().set(PlanList(plans));
    });
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
        return Err(format!("price changed: {} e8s, max allowed {max_price_e8s} e8s", quote.price_e8s));
    }

    let now = ic_cdk::api::time();
    let fee = ledger_fee();
    let args = TransferFromArgs {
        spender_subaccount: None,
        from: account(buyer),
        to: account(ic_cdk::api::canister_self()),
        amount: Nat::from(quote.price_e8s),
        fee: Some(Nat::from(fee)),
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
                quote.price_e8s + fee
            ),
            TransferFromError::InsufficientFunds { balance } => format!("insufficient funds: {balance} e8s"),
            other => format!("transfer_from error: {other:?}"),
        })?;

    let plan = quote.plan;
    let purchase_id = update_settings(|s| {
        let id = s.next_purchase;
        s.next_purchase += 1;
        id
    });
    PURCHASES.with(|m| {
        m.borrow_mut().insert(purchase_id, PlanPurchase {
            id: purchase_id,
            buyer,
            plan_id: plan.id.clone(),
            price_e8s: quote.price_e8s,
            block,
            timestamp_ns: now,
        })
    });

    let key = buyer.to_text();
    let mut sub = SUBSCRIPTIONS.with(|m| m.borrow().get(&key)).unwrap_or(Subscription {
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
    sub.expires_at_ns += plan.period_days as u64 * NS_PER_DAY;
    SUBSCRIPTIONS.with(|m| m.borrow_mut().insert(key, sub.clone()));
    Ok(sub)
}

#[query]
fn subscription_of(owner: Principal) -> Option<Subscription> {
    SUBSCRIPTIONS.with(|m| m.borrow().get(&owner.to_text()))
}

#[query]
fn purchases_of(owner: Principal) -> Vec<PlanPurchase> {
    PURCHASES.with(|m| m.borrow().iter().map(|e| e.value()).filter(|p| p.buyer == owner).collect())
}

/// Checks the plan of `owner` and takes `count` credits of `kind`.
/// `bytes` is the photo size, checked against the plan limit.
fn spend_credits(owner: Principal, kind: UsageKind, count: u64, bytes: u64) -> Result<Subscription, String> {
    let now = ic_cdk::api::time();
    let key = owner.to_text();
    let mut sub = SUBSCRIPTIONS.with(|m| m.borrow().get(&key)).ok_or("no plan: buy a plan first")?;
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
    SUBSCRIPTIONS.with(|m| m.borrow_mut().insert(key, sub.clone()));
    Ok(sub)
}

/// Spends plan credits for a mint done elsewhere. Controller only (MarkICP backend).
#[update]
fn consume_mints(owner: Principal, kind: UsageKind, count: u64, bytes: u64) -> Result<Subscription, String> {
    require_controller()?;
    spend_credits(owner, kind, count, bytes)
}

// ── NFTs (ICRC-7 subset) ────────────────────────────────────────────────────
//
// Photo mint: `create_upload` → `append_chunk` × N (≤ 1.9 MB each, ingress limit)
// → `mint_uploaded(upload_id, metadata, copies)`. `copies > 1` makes editions that
// share one stored image. Credits are taken inside the mint call, so a token
// exists only if the plan paid for it. The canister pays the cycles (reverse gas).

const MAX_METADATA_BYTES: usize = 8 * 1024;
const MAX_COPIES: u32 = 1_000;

#[derive(CandidType, Serialize, Deserialize, Clone)]
struct Token {
    owner: Principal,
    metadata: String,
    image_id: Option<u64>,
    minted_at_ns: u64,
    /// Copy number within one edition mint ("5 of 100"); None for single mints.
    /// Defaulted so tokens stored before editions were numbered still load.
    #[serde(default)]
    edition: Option<u32>,
    #[serde(default)]
    edition_total: Option<u32>,
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

fn check_metadata(metadata: &str) -> Result<(), String> {
    if metadata.len() > MAX_METADATA_BYTES {
        return Err(format!("metadata over {MAX_METADATA_BYTES} bytes"));
    }
    Ok(())
}

/// `numbered`: the tokens are copies of one edition and get "i of n" on-chain.
fn insert_tokens(owner: Principal, metadata: Vec<String>, image_id: Option<u64>, now: u64, numbered: bool) -> Vec<u64> {
    let total = metadata.len() as u32;
    let numbered = numbered && total > 1;
    let first = update_settings(|s| {
        let first = s.next_token;
        s.next_token += metadata.len() as u64;
        first
    });
    metadata
        .into_iter()
        .enumerate()
        .map(|(i, metadata)| {
            let id = first + i as u64;
            let token = Token {
                owner,
                metadata,
                image_id,
                minted_at_ns: now,
                edition: numbered.then_some(i as u32 + 1),
                edition_total: numbered.then_some(total),
            };
            TOKENS.with(|m| m.borrow_mut().insert(id, token));
            OWNER_TOKENS.with(|m| m.borrow_mut().insert(owner_key(&owner, id), ()));
            id
        })
        .collect()
}

#[update]
fn create_upload() -> Result<u64, String> {
    let owner = caller_not_anonymous()?;
    let now = ic_cdk::api::time();
    // Uploads cost cycles too: only callers with photo credits may start one.
    match SUBSCRIPTIONS.with(|m| m.borrow().get(&owner.to_text())) {
        Some(sub) if sub.expires_at_ns > now && sub.photo_mints_left > 0 => {}
        _ => return Err("no active plan with photo mints: buy a plan first".into()),
    }
    let id = NEXT_UPLOAD.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n += 1;
        id
    });
    UPLOADS.with(|u| u.borrow_mut().insert(id, Upload { owner, bytes: Vec::new() }));
    Ok(id)
}

/// Appends a chunk; returns the upload size so far.
#[update]
fn append_chunk(upload_id: u64, chunk: serde_bytes::ByteBuf) -> Result<u64, String> {
    let caller = caller_not_anonymous()?;
    let limit = SUBSCRIPTIONS.with(|m| m.borrow().get(&caller.to_text())).map(|s| s.max_photo_kb * 1024).unwrap_or(0);
    UPLOADS.with(|u| {
        let mut u = u.borrow_mut();
        let up = u.get_mut(&upload_id).ok_or("upload not found")?;
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
    let size = UPLOADS.with(|u| {
        let u = u.borrow();
        let up = u.get(&upload_id).ok_or("upload not found")?;
        if up.owner != owner {
            return Err("not your upload".to_string());
        }
        if up.bytes.is_empty() {
            return Err("upload is empty".to_string());
        }
        Ok(up.bytes.len() as u64)
    })?;
    let subscription = spend_credits(owner, UsageKind::Photo, copies as u64, size)?;

    let bytes = UPLOADS.with(|u| u.borrow_mut().remove(&upload_id)).unwrap().bytes;
    let image_id = update_settings(|s| {
        let id = s.next_image;
        s.next_image += 1;
        s.image_bytes += size;
        id
    });
    IMAGES.with(|m| m.borrow_mut().insert(image_id, Image(bytes)));

    let now = ic_cdk::api::time();
    let token_ids = insert_tokens(owner, vec![metadata; copies as usize], Some(image_id), now, true);
    Ok(MintResult { token_ids, image_bytes: size, subscription })
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
    let subscription = spend_credits(owner, UsageKind::Metadata, items.len() as u64, 0)?;
    let token_ids = insert_tokens(owner, items, None, ic_cdk::api::time(), false);
    Ok(MintResult { token_ids, image_bytes: 0, subscription })
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
    Nat::from(TOKENS.with(|m| m.borrow().len()))
}

fn nat_to_u64(n: &Nat) -> Option<u64> {
    n.0.to_string().parse().ok()
}

#[query]
fn icrc7_owner_of(ids: Vec<Nat>) -> Vec<Option<Account>> {
    TOKENS.with(|m| {
        let m = m.borrow();
        ids.iter()
            .map(|id| nat_to_u64(id).and_then(|id| m.get(&id)).map(|t| account(t.owner)))
            .collect()
    })
}

/// Token ids of `acc`, ascending, after `prev`, at most `take` (default 100, max 1000).
#[query]
fn icrc7_tokens_of(acc: Account, prev: Option<Nat>, take: Option<Nat>) -> Vec<Nat> {
    if acc.subaccount.is_some() {
        return vec![];
    }
    let start = prev.as_ref().and_then(nat_to_u64).map(|p| p + 1).unwrap_or(0);
    let take = take.as_ref().and_then(nat_to_u64).unwrap_or(100).min(1000) as usize;
    let prefix = format!("{}#", acc.owner.to_text());
    OWNER_TOKENS.with(|m| {
        m.borrow()
            .range(owner_key(&acc.owner, start)..)
            .map(|e| e.key().clone())
            .take_while(|k| k.starts_with(&prefix))
            .take(take)
            .filter_map(|k| k[prefix.len()..].parse::<u64>().ok().map(Nat::from))
            .collect()
    })
}

#[query]
fn icrc7_token_metadata(ids: Vec<Nat>) -> Vec<Option<Vec<(String, Value)>>> {
    ids.iter()
        .map(|id| {
            let t = TOKENS.with(|m| m.borrow().get(&nat_to_u64(id)?))?;
            let mut fields = vec![
                ("marki:metadata".to_string(), Value::Text(t.metadata.clone())),
                ("marki:minted_at_ns".to_string(), Value::Nat(Nat::from(t.minted_at_ns))),
            ];
            if let (Some(n), Some(total)) = (t.edition, t.edition_total) {
                fields.push(("marki:edition".to_string(), Value::Nat(Nat::from(n))));
                fields.push(("marki:edition_total".to_string(), Value::Nat(Nat::from(total))));
            }
            if let Some(img) = t.image_id.and_then(|i| IMAGES.with(|m| m.borrow().get(&i))) {
                fields.push(("marki:image_bytes".to_string(), Value::Nat(Nat::from(img.0.len()))));
            }
            Some(fields)
        })
        .collect()
}

#[query]
fn get_token(id: u64) -> Option<Token> {
    TOKENS.with(|m| m.borrow().get(&id))
}

#[query]
fn get_image(token_id: u64) -> Option<serde_bytes::ByteBuf> {
    let image_id = TOKENS.with(|m| m.borrow().get(&token_id))?.image_id?;
    IMAGES.with(|m| m.borrow().get(&image_id)).map(|img| serde_bytes::ByteBuf::from(img.0))
}

/// Cycle balance and storage. Read before and after a mint to see its real cost.
#[query]
fn nft_stats() -> NftStats {
    let s = settings();
    NftStats {
        cycles: Nat::from(ic_cdk::api::canister_cycle_balance()),
        tokens: TOKENS.with(|m| m.borrow().len()),
        images: IMAGES.with(|m| m.borrow().len()),
        image_bytes: s.image_bytes,
        heap_bytes: heap_bytes(),
    }
}

fn heap_bytes() -> u64 {
    #[cfg(target_arch = "wasm32")]
    return (core::arch::wasm32::memory_size(0) * 65_536) as u64;
    #[cfg(not(target_arch = "wasm32"))]
    0
}

#[query]
fn pricing() -> Pricing {
    let rate = RATE.with(|r| *r.borrow());
    let s = settings();
    Pricing {
        mint_call_cycles: MINT_CALL_CYCLES,
        mint_cycles_per_byte: MINT_CYCLES_PER_BYTE,
        batch_item_cycles: BATCH_ITEM_CYCLES,
        storage_cycles_per_gib_sec: STORAGE_CYCLES_PER_GIB_SEC as u64,
        markup_bps: MINT_MARKUP_BPS,
        xdr_usd: XDR_USD,
        xdr_permyriad_per_icp: rate.xdr_permyriad_per_icp,
        rate_timestamp_seconds: rate.timestamp_seconds,
        ledger: s.ledger,
        ledger_fee_e8s: s.ledger_fee_e8s,
    }
}

// ── Lifecycle ───────────────────────────────────────────────────────────────

#[ic_cdk::init]
fn init(arg: Option<InitArg>) {
    let arg = arg.unwrap_or_default();
    if let Some(ledger) = arg.ledger {
        update_settings(|s| s.ledger = ledger);
    }
    PLANS.with(|p| {
        p.borrow_mut().set(PlanList(default_plans()));
    });
}

/// Stable structures restore themselves; plans and settings are kept as they were.
#[ic_cdk::post_upgrade]
fn post_upgrade() {}

ic_cdk::export_candid!();

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T: Storable>(v: T) -> T {
        T::from_bytes(Cow::Owned(v.into_bytes()))
    }

    #[test]
    fn stored_records_roundtrip() {
        let p = Principal::from_text("2vxsx-fae").unwrap();
        let purchase = roundtrip(PlanPurchase {
            id: 7,
            buyer: p,
            plan_id: "start".into(),
            price_e8s: 123,
            block: Nat::from(u128::MAX),
            timestamp_ns: 9,
        });
        assert_eq!(purchase.buyer, p);
        assert_eq!(purchase.block, Nat::from(u128::MAX));
        let img = roundtrip(Image(vec![1, 2, 3, 255]));
        assert_eq!(img.0, vec![1, 2, 3, 255]);
        let s = roundtrip(Settings::default());
        assert_eq!(s.ledger, Principal::from_text(ICP_LEDGER).unwrap());
        assert_eq!(roundtrip(PlanList(default_plans())).0.len(), 3);
    }

    #[test]
    fn tokens_stored_before_numbering_still_load() {
        // A token as the live canister stored it before edition numbers existed.
        #[derive(Serialize)]
        struct OldToken { owner: Principal, metadata: String, image_id: Option<u64>, minted_at_ns: u64 }
        let mut bytes = Vec::new();
        ciborium::into_writer(&OldToken { owner: Principal::anonymous(), metadata: "{}".into(), image_id: Some(1), minted_at_ns: 7 }, &mut bytes).unwrap();
        let t = Token::from_bytes(Cow::Owned(bytes));
        assert_eq!((t.minted_at_ns, t.edition, t.edition_total), (7, None, None));
        let numbered = roundtrip(Token { edition: Some(5), edition_total: Some(100), ..t });
        assert_eq!((numbered.edition, numbered.edition_total), (Some(5), Some(100)));
    }

    #[test]
    fn owner_keys_sort_by_token_id() {
        let p = Principal::from_text("2vxsx-fae").unwrap();
        assert!(owner_key(&p, 9) < owner_key(&p, 10));
        assert!(owner_key(&p, 10).starts_with(&format!("{}#", p.to_text())));
    }
}
