// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, contracttype};

/// Stores all vesting schedule details for a single beneficiary.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BeneficiarySchedule {
    /// Total tokens to vest for this beneficiary.
    pub amount: i128,
    /// Ledger sequence at which vesting begins (cliff).
    pub cliff_ledger: u32,
    /// Ledger sequence at which all tokens are fully vested.
    pub end_ledger: u32,
    /// Tokens already claimed by the beneficiary.
    pub claimed: i128,
    /// Whether the schedule has been revoked by admin.
    pub revoked: bool,
    /// Ledger at which a pending revocation was requested (0 if none).
    pub revocation_requested_ledger: u32,
    /// Ledger at which a pending revocation finalizes (0 if none).
    pub revocation_finalize_ledger: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin address.
    Admin,
    /// Token contract address.
    Token,
    /// Vesting schedule for a specific beneficiary.
    Schedule(Address),
    /// Total tokens released early by admin (audit log).
    AdminReleased,
    /// Contract version number (`u32`).
    Version,
    /// Configurable revocation grace period in ledgers (`u32`).
    RevocationDelay,
}

/// Snapshot returned by `get_info`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct VestingInfo {
    pub token: Address,
    pub cliff_ledger: u32,
    pub end_ledger: u32,
    pub amount: i128,
    pub claimed: i128,
    pub revoked: bool,
    pub revocation_requested_ledger: u32,
    pub revocation_finalize_ledger: u32,
}