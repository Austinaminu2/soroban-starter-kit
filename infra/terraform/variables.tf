variable "environment" {
  description = "Deployment environment (testnet | mainnet)"
  type        = string
  default     = "testnet"

  validation {
    condition     = contains(["testnet", "mainnet"], var.environment)
    error_message = "environment must be 'testnet' or 'mainnet'."
  }
}

variable "network" {
  description = "Stellar network to target (testnet | mainnet | local)"
  type        = string
  default     = "testnet"

  validation {
    condition     = contains(["testnet", "mainnet", "local"], var.network)
    error_message = "network must be 'testnet', 'mainnet', or 'local'."
  }
}

variable "network_passphrase" {
  description = "Stellar network passphrase (only used for local networks; mainnet/testnet passphrases are derived from network variable)"
  type        = string
  sensitive   = true
  default     = "Test SDF Network ; September 2015"
}

variable "admin_key_path" {
  description = "File path to the admin account secret key (hex-encoded 32-byte seed)"
  type        = string
  default     = "~/.config/stellar/admin.key"
}

variable "token_name" {
  description = "Display name for the token contract"
  type        = string
  default     = "Soroban Token"
}

variable "token_symbol" {
  description = "Short ticker symbol for the token contract (e.g. XLM, USDC)"
  type        = string
  default     = "STK"
}

variable "cloud" {
  description = "Cloud provider for backend state storage (aws | gcp | azure)"
  type        = string
  default     = "aws"

  validation {
    condition     = contains(["aws", "gcp", "azure"], var.cloud)
    error_message = "cloud must be 'aws', 'gcp', or 'azure'."
  }
}
