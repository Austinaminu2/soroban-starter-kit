#!/usr/bin/env bash
# monitor-marketplace.sh — display a human-readable marketplace status summary
#
# Usage:
#   ./scripts/monitor-marketplace.sh [network] <CONTRACT_ID>
#   ./scripts/monitor-marketplace.sh testnet CABC...
#   ./scripts/monitor-marketplace.sh local CABC...
#
# Environment overrides:
#   STELLAR_RPC_URL            Override the default RPC endpoint
#   STELLAR_NETWORK_PASSPHRASE Override the default network passphrase
#   MARKETPLACE_PAGE_SIZE      Number of active listings to fetch per page (default: 10)
#
# What it shows:
#   - Payment token, royalty BPS, royalty recipient
#   - Paginated list of active listings (seller, NFT contract, token ID, price, expiry)
#   - WARNING banners for: listings past their expiry that have not yet been swept

set -euo pipefail

# ── color codes ──────────────────────────────────────────────────────────────
RED='\033[0;31m'
YELLOW='\033[0;33m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

# ── argument parsing ─────────────────────────────────────────────────────────
if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 [network] <CONTRACT_ID>" >&2
  echo "  network: testnet (default) | mainnet | local" >&2
  exit 1
fi

if [[ $# -eq 2 ]]; then
  NETWORK="$1"
  CONTRACT_ID="$2"
else
  NETWORK="testnet"
  CONTRACT_ID="$1"
fi

case "$NETWORK" in
  testnet)
    RPC_URL="${STELLAR_RPC_URL:-https://soroban-testnet.stellar.org}"
    PASSPHRASE="${STELLAR_NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"
    ;;
  mainnet)
    RPC_URL="${STELLAR_RPC_URL:-https://soroban.stellar.org}"
    PASSPHRASE="${STELLAR_NETWORK_PASSPHRASE:-Public Global Stellar Network ; September 2015}"
    ;;
  local)
    RPC_URL="${STELLAR_RPC_URL:-http://localhost:${LOCAL_RPC_PORT:-8000}}"
    PASSPHRASE="${STELLAR_NETWORK_PASSPHRASE:-Standalone Network ; February 2017}"
    ;;
  *)
    echo "Unknown network: $NETWORK (use testnet|mainnet|local)" >&2
    exit 1
    ;;
esac

PAGE_SIZE="${MARKETPLACE_PAGE_SIZE:-10}"

# ── helpers ──────────────────────────────────────────────────────────────────
STELLAR_ARGS=(
  --id "$CONTRACT_ID"
  --rpc-url "$RPC_URL"
  --network-passphrase "$PASSPHRASE"
  --network "$NETWORK"
)

invoke() {
  stellar contract invoke "${STELLAR_ARGS[@]}" -- "$@" 2>/dev/null || echo "N/A"
}

# Strip surrounding quotes from stellar CLI output
strip_quotes() {
  echo "$1" | tr -d '"'
}

# Parse a single JSON field value from a struct string.
parse_field() {
  local field="$1"
  local raw="$2"
  echo "$raw" | grep -o "\"${field}\":[^,}]*" | head -1 | cut -d':' -f2- | tr -d '" '
}

# Parse an optional JSON field (handles {"Some":value} / {"None":null} encoding).
parse_optional_field() {
  local field="$1"
  local raw="$2"
  local val
  val=$(echo "$raw" | grep -o "\"${field}\":[^,}]*" | head -1 | cut -d':' -f2- | tr -d ' ')
  if echo "$val" | grep -q '"None"'; then
    echo "none"
  else
    echo "$val" | grep -o '"Some":[^}]*' | cut -d':' -f2- | tr -d '"{}' || echo "$val"
  fi
}

# Fetch the current ledger sequence from the RPC endpoint.
get_current_ledger() {
  local response
  response=$(curl -sf --max-time 10 \
    -X POST "$RPC_URL" \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger","params":{}}' \
    2>/dev/null || echo "")
  if [[ -n "$response" ]]; then
    echo "$response" | grep -o '"sequence":[0-9]*' | head -1 | cut -d':' -f2
  else
    echo "0"
  fi
}

# ── fetch marketplace config ──────────────────────────────────────────────────
echo ""
echo -e "${BOLD}Marketplace Monitor — ${CYAN}${CONTRACT_ID}${RESET}"
echo -e "Network: ${BOLD}${NETWORK}${RESET}  RPC: ${RPC_URL}"
echo "────────────────────────────────────────────────────"

# The marketplace exposes config via storage; read key fields individually.
PAYMENT_TOKEN=$(invoke get_payment_token 2>/dev/null || echo "N/A")
ROYALTY_BPS=$(invoke get_royalty_bps 2>/dev/null || echo "N/A")
ROYALTY_RECIPIENT=$(invoke get_royalty_recipient 2>/dev/null || echo "N/A")

PAYMENT_TOKEN=$(strip_quotes "$PAYMENT_TOKEN")
ROYALTY_BPS=$(strip_quotes "$ROYALTY_BPS")
ROYALTY_RECIPIENT=$(strip_quotes "$ROYALTY_RECIPIENT")

if [[ "$PAYMENT_TOKEN" == "N/A" && "$ROYALTY_BPS" == "N/A" ]]; then
  echo -e "${RED}Contract not initialized or unreachable.${RESET}"
  echo "────────────────────────────────────────────────────"
  echo ""
  exit 0
fi

# Convert BPS to a percentage string for display.
royalty_pct() {
  local bps="$1"
  if [[ "$bps" == "N/A" || -z "$bps" ]]; then
    echo "N/A"
    return
  fi
  # Integer arithmetic: BPS / 100 = whole %, BPS % 100 = fractional
  local whole=$(( bps / 100 ))
  local frac=$(( bps % 100 ))
  if [[ "$frac" -eq 0 ]]; then
    echo "${whole}%"
  else
    printf "%d.%02d%%" "$whole" "$frac"
  fi
}

ROYALTY_PCT=$(royalty_pct "$ROYALTY_BPS")
CURRENT_LEDGER=$(get_current_ledger)

echo -e "Payment token:     ${BOLD}${PAYMENT_TOKEN}${RESET}"
echo -e "Royalty:           ${BOLD}${ROYALTY_PCT}${RESET} (${ROYALTY_BPS} bps)"
echo -e "Royalty recipient: ${BOLD}${ROYALTY_RECIPIENT}${RESET}"
echo -e "Current ledger:    ${BOLD}${CURRENT_LEDGER}${RESET}"
echo ""

# ── fetch active listings ─────────────────────────────────────────────────────
echo -e "${BOLD}Active Listings${RESET}"
echo "────────────────────────────────────────────────────"

CURSOR=0
TOTAL_ACTIVE=0
EXPIRED_COUNT=0

while true; do
  PAGE_RAW=$(stellar contract invoke "${STELLAR_ARGS[@]}" \
    -- get_active_listings \
    --cursor "$CURSOR" \
    --limit "$PAGE_SIZE" \
    2>/dev/null || echo "N/A")

  if [[ "$PAGE_RAW" == "N/A" || -z "$PAGE_RAW" ]]; then
    break
  fi

  # Extract each listing entry block; entries are objects inside the "listings" array.
  # We iterate by splitting on "},{"  after stripping the outer wrapper.
  LISTINGS_SEGMENT=$(echo "$PAGE_RAW" | grep -o '"listings":\[.*\]' | head -1)

  if [[ -z "$LISTINGS_SEGMENT" ]] || echo "$LISTINGS_SEGMENT" | grep -q '"listings":\[\]'; then
    break
  fi

  # Parse individual entries. Each entry has "id" and "listing" sub-object.
  # Use python if available for robust JSON parsing; otherwise fall back to grep.
  if command -v python3 &>/dev/null; then
    ENTRIES=$(python3 - "$PAGE_RAW" <<'PYEOF'
import sys, json, re

raw = sys.argv[1]
try:
    data = json.loads(raw)
except Exception:
    # Try to find the JSON object inside the raw output
    m = re.search(r'\{.*\}', raw, re.S)
    if m:
        data = json.loads(m.group(0))
    else:
        sys.exit(0)

listings = data.get("listings", [])
for entry in listings:
    lid = entry.get("id", "?")
    lst = entry.get("listing", {})
    seller = lst.get("seller", "N/A")
    nft = lst.get("nft_contract", "N/A")
    token_id = lst.get("token_id", "N/A")
    price = lst.get("price", "N/A")
    expires = lst.get("expires_at")
    active = lst.get("active", True)
    exp_str = str(expires) if expires is not None else "none"
    print(f"{lid}|{seller}|{nft}|{token_id}|{price}|{exp_str}|{active}")
PYEOF
    )
    while IFS='|' read -r LID SELLER NFT_CONTRACT TOKEN_ID PRICE EXPIRES ACTIVE; do
      [[ -z "$LID" ]] && continue
      TOTAL_ACTIVE=$(( TOTAL_ACTIVE + 1 ))
      # Determine expiry status
      EXPIRY_DISPLAY="none"
      EXPIRY_WARNING=""
      if [[ "$EXPIRES" != "none" && "$EXPIRES" != "null" && -n "$EXPIRES" ]]; then
        EXPIRY_DISPLAY="ledger ${EXPIRES}"
        if [[ "$CURRENT_LEDGER" -gt 0 && "$EXPIRES" -lt "$CURRENT_LEDGER" ]]; then
          EXPIRY_DISPLAY="${EXPIRES} ${RED}(EXPIRED)${RESET}"
          EXPIRY_WARNING=1
          EXPIRED_COUNT=$(( EXPIRED_COUNT + 1 ))
        fi
      fi
      printf "  ${BOLD}#%-6s${RESET} Seller: %-46s Price: %s\n" "$LID" "$SELLER" "$PRICE"
      printf "          NFT: %-46s Token ID: %s\n" "$NFT_CONTRACT" "$TOKEN_ID"
      printf "          Expiry: %b\n" "$EXPIRY_DISPLAY"
      echo ""
    done <<< "$ENTRIES"
  else
    # Fallback: simple grep-based parsing — print raw JSON entries
    echo "$LISTINGS_SEGMENT" | grep -o '"id":[0-9]*' | cut -d':' -f2 | while read -r LID; do
      TOTAL_ACTIVE=$(( TOTAL_ACTIVE + 1 ))
      echo -e "  Listing ${BOLD}#${LID}${RESET}"
    done
  fi

  # Advance cursor using next_cursor field; stop if null/absent.
  NEXT_CURSOR=$(echo "$PAGE_RAW" | grep -o '"next_cursor":[^,}]*' | head -1 | cut -d':' -f2 | tr -d ' "')
  if [[ -z "$NEXT_CURSOR" || "$NEXT_CURSOR" == "null" ]]; then
    break
  fi
  CURSOR="$NEXT_CURSOR"
done

if [[ "$TOTAL_ACTIVE" -eq 0 ]]; then
  echo -e "  ${CYAN}No active listings found.${RESET}"
  echo ""
fi

echo "────────────────────────────────────────────────────"
echo -e "Total active listings: ${BOLD}${TOTAL_ACTIVE}${RESET}"

# ── warning banners ───────────────────────────────────────────────────────────
if [[ "$EXPIRED_COUNT" -gt 0 ]]; then
  echo ""
  echo -e "${YELLOW}${BOLD}WARNING: ${EXPIRED_COUNT} listing(s) past their expiry.${RESET}"
  echo -e "${YELLOW}Sellers may call \`sweep_expired\` to reclaim those listings.${RESET}"
fi

echo "────────────────────────────────────────────────────"
echo ""
