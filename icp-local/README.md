# MarkICP local payments test

Local ICP network plus a `payments` canister that sells an NFT listing for ICP using ICRC-2 (approve + transfer_from).

The icp-cli local network already contains the ICP ledger (`ryjl3-tyaaa-aaaaa-aaaba-cai`) and seeds the `anonymous` identity with ICP, so no ledger setup is needed.

## Run

```bash
cd icp-local
icp network start -d      # local network on :8000
icp deploy                # builds and installs `payments`
./scripts/demo.sh         # full purchase: fund buyer, list, approve, buy, check balances
icp network stop          # when done
```

## Purchase flow

1. Seller: `create_listing(nft_id, price_e8s)` returns a listing id.
2. Buyer: `icrc2_approve` on the ledger with the `payments` canister as spender, for `price + 0.0001 ICP` (ledger fee).
   CLI: `icp token approve 1.5001 <canister-id> --identity marki-buyer`.
3. Buyer: `buy(listing_id)`. The canister pulls the price into escrow, keeps 2.5 % platform fee, and pays the seller `price - fee - 0.0001 ICP`.

The listing is locked before the first ledger call, so a second `buy` fails with `listing already sold`.

## Plans (what companies pay for)

A company buys a plan, not single mints. Default plans (`default_plans` in `src/lib.rs`):

| Plan | Price / 30 days | Photo NFTs | Metadata NFTs | Photo limit | Worst-case cost |
|---|---|---|---|---|---|
| Старт | $9 | 100 | 1 000 | 1 MB | ~$0.84 |
| Бізнес | $49 | 1 000 | 10 000 | 2 MB | ~$16.8 |
| Про | $199 | 10 000 | 100 000 | 2 MB | ~$168 |

Worst-case cost = every photo at the size limit plus 1 year of storage (`max_cost_cycles` in `plans()`).
A typical 200 KB photo costs ~6x less.

Flow:

1. `plans()` returns plans priced in ICP (USD price / 1.36643 USD per XDR / CMC ICP-XDR rate). Call `refresh_rate()` first.
2. Company: `icrc2_approve` for the price + 0.0001 ICP, then `buy_plan(plan_id, max_price_e8s)`.
   Credits add up and the period extends; expired credits do not carry over.
3. Backend (a controller of the canister) calls `consume_mints(owner, variant { Photo }, count, bytes)` before each mint.
   It fails when the plan expired, credits are short, or the photo is over the plan limit.
4. `set_plans(vec Plan)` (controller only) changes prices and contents.

CLI test:

```bash
icp token approve 1.95 <canister-id> --identity marki-buyer
icp canister call payments buy_plan '("start", 190000000 : nat64)' --identity marki-buyer
icp canister call payments consume_mints '(principal "<buyer>", variant { Photo }, 3 : nat64, 500000 : nat64)'
```

`quote_mint` still prices single mints from the measured cycles model: use it to check plan margins.

## NFT minting (ICRC-7 subset)

The same canister mints NFTs, so plan credits and the mint happen in one call:

1. `create_upload()` — needs an active plan with photo mints.
2. `append_chunk(upload_id, blob)` × N — ≤ 1.9 MB per call; total checked against the plan photo limit.
3. `mint_uploaded(upload_id, metadata_json, copies)` — takes `copies` photo credits and mints; editions share one image.
4. `mint_metadata(vec text)` — metadata-only NFTs, takes metadata credits.

Reads: `icrc7_name`, `icrc7_symbol`, `icrc7_total_supply`, `icrc7_owner_of`, `icrc7_tokens_of`,
`icrc7_token_metadata`, `get_token`, `get_image`, `nft_stats` (cycle balance: read before and after a mint to see its real cost).

Measured: a 35.2 KB photo costs ~94M cycles (~$0.00013) across the three calls.

## Full local test

```bash
# 1. ICP network + canister
cd icp-local && icp network start -d && icp deploy
# 2. Rust API (api/), port 8090
cd ../api && cargo run
# 3. Frontend with .env.icplocal (local ICP + local API)
cd ../web && npm run dev:icp
```

Then in the app:

1. «Тарифи» → «Dev-гаманець» → fund it: `icp token transfer 10 <principal> --identity anonymous` → buy «Старт».
2. «NFT» → «Створити NFT» → photo → «Випустити NFT».
   The wizard mints on-chain first (fails without a plan), then saves the NFT in the API with
   `mintAddress = icp:<canister>:<token>`. The success screen shows tokens, cycles spent and credits left.

Plug does not work well with a local network; use the dev wallet locally.

## Notes

- State is heap-only: an upgrade wipes listings, receipts and subscriptions (plans reset to defaults). Test use only.
- For mainnet tests without real money, switch `ICP_LEDGER` in `src/lib.rs` to the TESTICP ledger `xafvr-biaaa-aaaai-aql5q-cai` and get tokens from https://faucet.internetcomputer.org.
- Candid UI: printed by `icp deploy`.
