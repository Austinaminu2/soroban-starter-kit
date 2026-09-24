#!/usr/bin/env bash
# scripts/contract-size-report.sh — Build each contract's optimised WASM and report its size.
#
# Usage:
#   ./scripts/contract-size-report.sh [--check <limit_kb>]
#
# Options:
#   --check <limit_kb>   Exit non-zero if any contract exceeds <limit_kb> KB (default: no limit).
#
# Soroban imposes resource limits sensitive to contract size, so tracking WASM
# sizes in CI makes size regressions immediately visible.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

log()  { echo -e "\033[1;34m[size-report]\033[0m $*"; }
ok()   { echo -e "\033[1;32m[ok]\033[0m $*"; }
warn() { echo -e "\033[1;33m[warn]\033[0m $*"; }
die()  { echo -e "\033[1;31m[error]\033[0m $*" >&2; exit 1; }

# ── Argument parsing ──────────────────────────────────────────────────────────
LIMIT_KB=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check)
      shift
      [[ $# -gt 0 ]] || die "--check requires a numeric KB argument"
      LIMIT_KB="$1"
      ;;
    *) warn "Unknown argument: $1" ;;
  esac
  shift
done

# ── Prerequisite checks ───────────────────────────────────────────────────────
command -v stellar &>/dev/null || die "stellar-cli not found — run ./scripts/setup.sh first"
rustup target list --installed | grep -q "wasm32-unknown-unknown" \
  || die "wasm32-unknown-unknown target not installed — run: rustup target add wasm32-unknown-unknown"

# ── Build & measure ───────────────────────────────────────────────────────────
CONTRACTS_DIR="$ROOT/contracts"
RELEASE_DIR="$ROOT/target/wasm32-unknown-unknown/release"

exceeded=()
printf "\n%-30s %10s\n" "CONTRACT" "SIZE"
printf "%-30s %10s\n" "--------" "----"

for manifest in "$CONTRACTS_DIR"/*/Cargo.toml; do
  contract_dir="$(dirname "$manifest")"
  contract_name="$(basename "$contract_dir")"

  log "Building $contract_name..."
  stellar contract build --manifest-path "$manifest" --quiet 2>/dev/null \
    || { warn "Build failed for $contract_name — skipping"; continue; }

  # Resolve WASM file — stellar-cli uses underscores in the output filename
  wasm_name="${contract_name//-/_}.wasm"
  wasm_path="$RELEASE_DIR/$wasm_name"

  # Fallback: find the most recently modified wasm matching the contract
  if [[ ! -f "$wasm_path" ]]; then
    wasm_path="$(ls -t "$RELEASE_DIR"/*"${contract_name//-/_}"*.wasm 2>/dev/null | head -n1 || true)"
  fi

  if [[ -z "$wasm_path" || ! -f "$wasm_path" ]]; then
    warn "Could not locate WASM for $contract_name — skipping"
    continue
  fi

  size_bytes="$(wc -c < "$wasm_path")"
  size_kb="$(echo "scale=2; $size_bytes / 1024" | bc)"

  printf "%-30s %7s KB\n" "$contract_name" "$size_kb"

  if [[ -n "$LIMIT_KB" ]] && (( $(echo "$size_kb > $LIMIT_KB" | bc -l) )); then
    exceeded+=("$contract_name (${size_kb} KB > ${LIMIT_KB} KB limit)")
  fi
done

printf "\n"

if [[ ${#exceeded[@]} -gt 0 ]]; then
  warn "The following contracts exceed the ${LIMIT_KB} KB limit:"
  for entry in "${exceeded[@]}"; do
    echo "  ❌  $entry"
  done
  exit 1
fi

ok "Size report complete."
