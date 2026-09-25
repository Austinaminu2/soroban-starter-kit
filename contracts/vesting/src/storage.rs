// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, contracttype};

/// Stores all vesting schedule details for a single beneficiary and token.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct BeneficiarySchedule {
    /// Token contract address for this schedule.
    pub token: Address,
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
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin address.
    Admin,
    /// Token contract address.
    Token,
    /// Vesting schedule for a specific beneficiary and token.
    Schedule(Address, Address),
    /// Total tokens released early by admin (audit log).
    AdminReleased,
    /// Contract version number (`u32`).
    Version,
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
}