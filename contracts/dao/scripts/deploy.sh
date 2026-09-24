#!/bin/bash

# Soroban DAO Governance Contract Deployment Script
# Usage: ./deploy.sh [network]
# Example: ./deploy.sh testnet

set -e

NETWORK=${1:-testnet}
CONTRACT_NAME="soroban-dao-template"

echo "Deploying DAO Contract to $NETWORK..."

echo "Building contract..."
stellar contract build

echo "Deploying to $NETWORK..."
CONTRACT_ID=$(stellar contract deploy \
    --wasm target/wasm32-unknown-unknown/release/${CONTRACT_NAME}.wasm \
    --network $NETWORK)

echo "DAO contract deployed!"
echo "Contract ID: $CONTRACT_ID"

CONTRACT_IDS_FILE="../../../.contract-ids"
if [ ! -f "$CONTRACT_IDS_FILE" ]; then
    echo "{\"network\": \"$NETWORK\", \"contracts\": {\"dao\": \"$CONTRACT_ID\"}}" > "$CONTRACT_IDS_FILE"
else
    TEMP_FILE="${CONTRACT_IDS_FILE}.tmp"
    jq --arg network "$NETWORK" --arg contract_id "$CONTRACT_ID" \
        '.network = $network | .contracts.dao = $contract_id' \
        "$CONTRACT_IDS_FILE" > "$TEMP_FILE"
    mv "$TEMP_FILE" "$CONTRACT_IDS_FILE"
fi

# Example initialization (uncomment to use):
# ADMIN_ADDRESS="G..."
# TOKEN_ADDRESS="C..."         # governance token contract
# VOTING_PERIOD=17280          # ~24h at 5s/ledger
# QUORUM=1000000000            # minimum total votes (token units)
# QUORUM_BPS=0                 # % of total supply required (0 = disabled, 5000 = 50%)

# stellar contract invoke \
#     --id $CONTRACT_ID \
#     --network $NETWORK \
#     -- initialize \
#     --admin $ADMIN_ADDRESS \
#     --token $TOKEN_ADDRESS \
#     --voting_period $VOTING_PERIOD \
#     --quorum $QUORUM \
#     --quorum_bps $QUORUM_BPS

echo "DAO contract ready for use!"
echo "Save this Contract ID: $CONTRACT_ID"
