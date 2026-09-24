#!/usr/bin/env bash
# monitor-lottery.sh — display a human-readable lottery status summary
#
# Usage:
#   ./scripts/monitor-lottery.sh [network] <CONTRACT_ID>
#   ./scripts/monitor-lottery.sh testnet CABC...
#   ./scripts/monitor-lottery.sh local CABC...
#
# Environment overrides:
#   STELLAR_RPC_URL            Override the default RPC endpoint
#   STELLAR_NETWORK_PASSPHRASE Override the default network passphrase
#
# What it shows:
#   - Admin, token, ticket price
#   - Lottery state (Open / Committed / Drawn)
#   - Participant count
#   - Commit reveal deadline and approximate time remaining (Committed state)
#   - Winner(s) with prize amounts (Drawn state)
#   - WARNING banners for: reveal deadline overdue without draw (refunds available)

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

# Convert remaining ledgers to approximate human-readable time (5 s per ledger)
ledgers_to_time() {
  local ledgers="$1"
  if [[ "$ledgers" == "N/A" ]]; then
    echo "N/A"
    return
  fi
  local abs_ledgers="${ledgers#-}"
  local total_seconds=$(( abs_ledgers * 5 ))
  local days=$(( total_seconds / 86400 ))
  local hours=$(( (total_seconds % 86400) / 3600 ))
  local minutes=$(( (total_seconds % 3600) / 60 ))

  if [[ "$ledgers" -lt 0 ]]; then
    echo "${days}d ${hours}h ${minutes}m ago (OVERDUE)"
  elif [[ "$days" -gt 0 ]]; then
    echo "${days}d ${hours}h ${minutes}m remaining"
  elif [[ "$hours" -gt 0 ]]; then
    echo "${hours}h ${minutes}m remaining"
  else
    echo "${minutes}m remaining"
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

# Parse a JSON field from a struct output string.
parse_field() {
  local field="$1"
  local raw="$2"
  echo "$raw" | grep -o "\"${field}\":[^,}]*" | head -1 | cut -d':' -f2- | tr -d '" '
}

# Map raw state string to a colored display label.
state_display() {
  local raw
  raw=$(strip_quotes "$1")
  case "$raw" in
    Open)       echo "${CYAN}Open (accepting tickets)${RESET}" ;;
    Committed)  echo "${YELLOW}Committed (awaiting draw)${RESET}" ;;
    Drawn)      echo "${GREEN}Drawn (complete)${RESET}" ;;
    N/A)        echo "${RED}Not initialized${RESET}" ;;
    *)          echo "$raw" ;;
  esac
}

# ── fetch lottery data ───────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}Lottery Monitor — ${CYAN}${CONTRACT_ID}${RESET}"
echo -e "Network: ${BOLD}${NETWORK}${RESET}  RPC: ${RPC_URL}"
echo "────────────────────────────────────────────────────"

INFO_RAW=$(stellar contract invoke "${STELLAR_ARGS[@]}" -- get_info 2>/dev/null || echo "N/A")

if [[ "$INFO_RAW" == "N/A" ]]; then
  echo -e "${RED}Contract not initialized or unreachable.${RESET}"
  echo "────────────────────────────────────────────────────"
  echo ""
  exit 0
fi

ADMIN=$(parse_field "admin" "$INFO_RAW")
TOKEN=$(parse_field "token" "$INFO_RAW")
TICKET_PRICE=$(parse_field "ticket_price" "$INFO_RAW")
STATE_RAW=$(parse_field "state" "$INFO_RAW")

# Participant count: the participants field is an array; count commas+1 as a proxy,
# or fall back to 0 if empty.
PARTICIPANTS_SEGMENT=$(echo "$INFO_RAW" | grep -o '"participants":\[[^]]*\]' | head -1)
if echo "$PARTICIPANTS_SEGMENT" | grep -q '"'; then
  PARTICIPANT_COUNT=$(echo "$PARTICIPANTS_SEGMENT" | grep -o '"G[^"]*"' | wc -l | tr -d ' ')
else
  PARTICIPANT_COUNT=0
fi

STATE_DISPLAY=$(state_display "$STATE_RAW")
CURRENT_LEDGER=$(get_current_ledger)

# ── print core summary ────────────────────────────────────────────────────────
echo -e "State:             ${STATE_DISPLAY}"
echo ""
echo -e "Admin:             ${BOLD}${ADMIN:-N/A}${RESET}"
echo -e "Token:             ${BOLD}${TOKEN:-N/A}${RESET}"
echo -e "Ticket price:      ${BOLD}${TICKET_PRICE:-N/A}${RESET} (base units)"
echo -e "Participants:      ${BOLD}${PARTICIPANT_COUNT}${RESET} ticket(s) sold"
echo -e "Current ledger:    ${BOLD}${CURRENT_LEDGER}${RESET}"

# ── committed-state: show reveal deadline ────────────────────────────────────
STATE_CLEAN=$(strip_quotes "$STATE_RAW")
if [[ "$STATE_CLEAN" == "Committed" ]]; then
  echo ""
  REVEAL_DEADLINE=$(invoke get_reveal_deadline 2>/dev/null || echo "N/A")
  REVEAL_DEADLINE=$(strip_quotes "$REVEAL_DEADLINE")
  if [[ "$REVEAL_DEADLINE" != "N/A" && "$CURRENT_LEDGER" -gt 0 ]]; then
    REMAINING=$(( REVEAL_DEADLINE - CURRENT_LEDGER ))
  else
    REMAINING="N/A"
  fi
  TIME_DISPLAY=$(ledgers_to_time "${REMAINING}")
  echo -e "Reveal deadline:   ledger ${BOLD}${REVEAL_DEADLINE}${RESET}"
  echo -e "Time to reveal:    ${BOLD}${TIME_DISPLAY}${RESET}"

  if [[ "$REMAINING" != "N/A" && "$REMAINING" -lt 0 ]]; then
    echo ""
    echo -e "${RED}${BOLD}WARNING: Reveal deadline has passed without a draw.${RESET}"
    echo -e "${YELLOW}Ticket buyers may now call \`claim_refund\` to recover their ticket price.${RESET}"
  fi
fi

# ── drawn-state: show winner(s) ───────────────────────────────────────────────
if [[ "$STATE_CLEAN" == "Drawn" ]]; then
  echo ""
  WINNERS_RAW=$(stellar contract invoke "${STELLAR_ARGS[@]}" -- get_winners 2>/dev/null || echo "N/A")
  if [[ "$WINNERS_RAW" != "N/A" ]]; then
    echo -e "${GREEN}${BOLD}Winners:${RESET}"
    # Each address is a quoted string inside the array; print one per line.
    echo "$WINNERS_RAW" | grep -o '"G[^"]*"' | tr -d '"' | nl -w2 -s'. ' | while IFS= read -r line; do
      echo -e "  ${BOLD}${line}${RESET}"
    done
  else
    WINNER_RAW=$(invoke get_winner)
    WINNER=$(strip_quotes "$WINNER_RAW")
    echo -e "${GREEN}${BOLD}Winner: ${WINNER}${RESET}"
  fi
fi

# ── open-state reminder ───────────────────────────────────────────────────────
if [[ "$STATE_CLEAN" == "Open" ]]; then
  echo ""
  echo -e "${CYAN}Lottery is open. Players may call \`buy_ticket\` to participate.${RESET}"
fi

echo "────────────────────────────────────────────────────"
echo ""
