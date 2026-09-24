// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, contracttype};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin address.
    Admin,
    /// Token that users stake.
    StakeToken,
    /// Token distributed as rewards (may be the same as StakeToken).
    RewardToken,
    /// Total tokens currently staked across all stakers.
    TotalStaked,
    /// Total reward tokens deposited and not yet claimed.
    TotalRewards,
    /// Reward-per-token accumulator (scaled by REWARD_SCALE).
    RewardPerTokenStored,
    /// Per-staker: amount staked.
    Stake(Address),
    /// Per-staker: reward-per-token snapshot at last update.
    RewardPerTokenPaid(Address),
    /// Per-staker: accrued but unclaimed rewards.
    Rewards(Address),
    /// Contract version number (`u32`).
    Version,
    /// Per-staker: whether auto-compounding is enabled (`bool`).
    Compounding(Address),
    /// Unbonding delay in ledgers; 0 means immediate withdrawal is allowed.
    UnbondingPeriod,
    /// Per-staker: pending unbond request.
    UnbondRequest(Address),
    /// Address that receives slashed tokens (treasury / burn).
    SlashDestination,
}

/// Scaling factor for reward-per-token fixed-point arithmetic.
/// Using 1e12 gives enough precision for typical token amounts.
pub const REWARD_SCALE: i128 = 1_000_000_000_000;

/// Holds the state of an unbonding request for a staker.
#[contracttype]
#[derive(Clone, Debug)]
pub struct UnbondRequest {
    /// Amount of stake tokens queued for withdrawal.
    pub amount: i128,
    /// Ledger sequence after which `withdraw` becomes valid.
    pub available_at: u32,
}
