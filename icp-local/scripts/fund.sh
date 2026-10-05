#!/usr/bin/env bash
# Fund a wallet on the local network with test ICP from the seeded anonymous identity.
# Usage: ./scripts/fund.sh <principal> [amount, default 10]
set -euo pipefail
cd "$(dirname "$0")/.."
if [ $# -lt 1 ]; then
  echo "Usage: $0 <principal> [amount]" >&2
  echo "The principal is shown on the «Тарифи» page after «Dev-гаманець»." >&2
  exit 1
fi
icp token transfer "${2:-10}" "$1" --identity anonymous
icp token balance --identity anonymous >/dev/null
