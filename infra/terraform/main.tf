terraform {
  required_version = ">= 1.5"

  required_providers {
    null  = { source = "hashicorp/null",  version = ">= 3.0" }
    local = { source = "hashicorp/local", version = ">= 2.0" }
  }

  # Backend config is supplied at init time via -backend-config flags in CI.
  # Required secrets: TF_STATE_BUCKET (S3 bucket name), TF_LOCK_TABLE (DynamoDB table name).
  backend "s3" {
    # bucket, key, region, and dynamodb_table are injected by CI via -backend-config
    encrypt = true
  }
}

locals {
  rpc_url = (
    var.network == "mainnet" ? "https://soroban.stellar.org" :
    var.network == "testnet" ? "https://soroban-testnet.stellar.org" :
    "http://localhost:8000"
  )

  network_passphrase = (
    var.network == "mainnet" ? "Public Global Stellar Network ; September 2015" :
    var.network == "testnet" ? "Test SDF Network ; September 2015" :
    var.network_passphrase
  )
}

# ── Stellar account setup ─────────────────────────────────────────────────────
resource "null_resource" "stellar_account_setup" {
  triggers = {
    admin_key_path = var.admin_key_path
    network        = var.network
    environment    = var.environment
  }

  provisioner "local-exec" {
    interpreter = ["bash", "-c"]
    command     = <<-EOT
      set -euo pipefail
      cat '${var.admin_key_path}' | stellar keys add admin --secret-key - --overwrite
      stellar keys address admin | tr -d '\n' > '${path.module}/.admin_address'
    EOT
  }
}

data "local_file" "admin_address" {
  count      = fileexists("${path.module}/.admin_address") ? 1 : 0
  filename   = "${path.module}/.admin_address"
  depends_on = [null_resource.stellar_account_setup]
}

# ── Fund admin account on testnet via Friendbot ───────────────────────────────
resource "null_resource" "fund_testnet_account" {
  count      = var.network == "testnet" ? 1 : 0
  depends_on = [null_resource.stellar_account_setup]

  triggers = {
    admin_key_path = var.admin_key_path
    environment    = var.environment
  }

  provisioner "local-exec" {
    interpreter = ["bash", "-c"]
    command     = <<-EOT
      ADDR=$(cat '${path.module}/.admin_address')
      curl -sSf "https://friendbot.stellar.org?addr=$${ADDR}" > /dev/null
      echo "Funded $${ADDR} via Friendbot"
    EOT
  }
}

# ── Token contract deployment ─────────────────────────────────────────────────
resource "null_resource" "deploy_token_contract" {
  depends_on = [null_resource.fund_testnet_account, null_resource.stellar_account_setup]

  triggers = {
    network      = var.network
    environment  = var.environment
    token_name   = var.token_name
    token_symbol = var.token_symbol
  }

  provisioner "local-exec" {
    interpreter = ["bash", "-c"]
    command     = <<-EOT
      set -euo pipefail
      WASM=$(find '${path.root}/../../target/wasm32-unknown-unknown/release' \
        \( -name 'soroban_token_contract.wasm' -o -name 'token.wasm' \) \
        2>/dev/null | head -1)
      [[ -n "$WASM" ]] || { echo "Token WASM not found — build contracts first"; exit 1; }
      stellar contract deploy \
        --wasm "$WASM" \
        --source admin \
        --network '${var.network}' \
        --rpc-url '${local.rpc_url}' \
        --network-passphrase '${local.network_passphrase}' \
        | tr -d '\n' > '${path.module}/.token_contract_id'
      echo "Token contract deployed: $(cat '${path.module}/.token_contract_id')"
    EOT
  }
}

data "local_file" "token_contract_id" {
  count      = fileexists("${path.module}/.token_contract_id") ? 1 : 0
  filename   = "${path.module}/.token_contract_id"
  depends_on = [null_resource.deploy_token_contract]
}

# ── Escrow contract deployment ────────────────────────────────────────────────
resource "null_resource" "deploy_escrow_contract" {
  depends_on = [null_resource.fund_testnet_account, null_resource.stellar_account_setup]

  triggers = {
    network     = var.network
    environment = var.environment
  }

  provisioner "local-exec" {
    interpreter = ["bash", "-c"]
    command     = <<-EOT
      set -euo pipefail
      WASM=$(find '${path.root}/../../target/wasm32-unknown-unknown/release' \
        \( -name 'soroban_escrow_contract.wasm' -o -name 'escrow.wasm' \) \
        2>/dev/null | head -1)
      [[ -n "$WASM" ]] || { echo "Escrow WASM not found — build contracts first"; exit 1; }
      stellar contract deploy \
        --wasm "$WASM" \
        --source admin \
        --network '${var.network}' \
        --rpc-url '${local.rpc_url}' \
        --network-passphrase '${local.network_passphrase}' \
        | tr -d '\n' > '${path.module}/.escrow_contract_id'
      echo "Escrow contract deployed: $(cat '${path.module}/.escrow_contract_id')"
    EOT
  }
}

data "local_file" "escrow_contract_id" {
  count      = fileexists("${path.module}/.escrow_contract_id") ? 1 : 0
  filename   = "${path.module}/.escrow_contract_id"
  depends_on = [null_resource.deploy_escrow_contract]
}

# ── Token contract initialisation ─────────────────────────────────────────────
resource "null_resource" "init_token_contract" {
  depends_on = [null_resource.deploy_token_contract]

  triggers = {
    token_name   = var.token_name
    token_symbol = var.token_symbol
    environment  = var.environment
  }

  provisioner "local-exec" {
    interpreter = ["bash", "-c"]
    command     = <<-EOT
      set -euo pipefail
      TOKEN_ID=$(cat '${path.module}/.token_contract_id')
      ADMIN=$(cat '${path.module}/.admin_address')
      stellar contract invoke \
        --id "$TOKEN_ID" \
        --source admin \
        --network '${var.network}' \
        --rpc-url '${local.rpc_url}' \
        --network-passphrase '${local.network_passphrase}' \
        -- initialize \
        --admin "$ADMIN" \
        --decimal 7 \
        --name '${var.token_name}' \
        --symbol '${var.token_symbol}'
      echo "Token contract initialised"
    EOT
  }
}
