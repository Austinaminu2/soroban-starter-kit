// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, contracttype, Vec};

/// Represents a single tranche in a multi-tranche release schedule.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ReleaseTranche {
    /// Ledger sequence at which this tranche becomes available for release.
    pub release_ledger: u32,
    /// Amount of tokens in this tranche.
    pub amount: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin address.
    Admin,
    /// Token contract address.
    Token,
    /// Beneficiary address.
    Beneficiary,
    /// Deprecated: single release ledger (kept for backwards compatibility).
    ReleaseLedger,
    /// Deprecated: single amount (kept for backwards compatibility).
    Amount,
    /// Timelock state.
    State,
    /// List of release tranches for multi-tranche releases.
    Tranches,
    /// Bitmask tracking which tranches have been released (index -> true/false).
    ReleasedTranches,
}

#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimelockState {
    Active = 0,
    Released = 1,
    Cancelled = 2,
}

impl core::fmt::Display for TimelockState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            TimelockState::Active => "active",
            TimelockState::Released => "released",
            TimelockState::Cancelled => "cancelled",
        })
    }
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct TimelockInfo {
    pub admin: Address,
    pub token: Address,
    pub beneficiary: Address,
    /// Deprecated: single release ledger (for backwards compatibility).
    pub release_ledger: u32,
    /// Deprecated: single amount (for backwards compatibility).
    pub amount: i128,
    pub state: TimelockState,
    /// List of tranches (empty for legacy single-tranche timelocks).
    pub tranches: Vec<ReleaseTranche>,
}
