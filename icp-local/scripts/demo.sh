#!/usr/bin/env bash
# End-to-end test of a crypto (ICP) purchase on the local network.
# Needs: `icp network start -d` and `icp deploy` already done in icp-local/.
set -euo pipefail
cd "$(dirname "$0")/.."

PRICE_E8S=${PRICE_E8S:-150000000} # 1.5 ICP
NFT_ID=${NFT_ID:-marki-nft-001}

ensure_identity() {
  icp identity list | grep -q " $1 " || icp identity new "$1" --storage plaintext >/dev/null
}
ensure_identity marki-seller
ensure_identity marki-buyer

SELLER=$(icp identity principal --identity marki-seller)
BUYER=$(icp identity principal --identity marki-buyer)
CANISTER=$(icp canister status payments 2>/dev/null | awk "/Canister Id/ {print \$NF; exit}")
echo "seller:   $SELLER"
echo "buyer:    $BUYER"
echo "canister: $CANISTER"

balances() {
  echo "  seller   $(icp token balance --identity marki-seller)"
  echo "  buyer    $(icp token balance --identity marki-buyer)"
}

echo "== Fund buyer with 10 ICP from the seeded anonymous identity"
icp token transfer 10 "$BUYER" --identity anonymous -q >/dev/null
echo "== Balances before"
balances

echo "== Seller creates listing ($NFT_ID for $PRICE_E8S e8s)"
LISTING=$(icp canister call payments create_listing "(\"$NFT_ID\", $PRICE_E8S : nat64)" --identity marki-seller)
echo "  $LISTING"
LISTING_ID=$(echo "$LISTING" | grep -oE '[0-9]+' | head -1)

echo "== Buy without approve (must fail with insufficient allowance)"
icp canister call payments buy "($LISTING_ID : nat64)" --identity marki-buyer || true

echo "== Buyer approves canister for price + 0.0001 ICP ledger fee"
APPROVE=$(python3 -c "print(($PRICE_E8S + 10000) / 1e8)")
icp token approve "$APPROVE" "$CANISTER" --identity marki-buyer

echo "== Buyer buys"
icp canister call payments buy "($LISTING_ID : nat64)" --identity marki-buyer

echo "== Second buy (must fail: already sold)"
icp canister call payments buy "($LISTING_ID : nat64)" --identity marki-buyer || true

echo "== Balances after"
balances
echo "  platform $(icp canister call payments platform_balance '()') e8s"
