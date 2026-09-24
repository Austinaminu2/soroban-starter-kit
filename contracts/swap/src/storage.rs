// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, contracttype};

/// Instance-storage keys.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    SwapCount,
    BasketSwapCount,
    Initialized,
    Admin,
    Treasury,
    FeeBps,
}

/// Persistent-storage key for individual swaps.
#[contracttype]
#[derive(Clone)]
pub enum SwapKey {
    Swap(u32),
    BasketSwap(u32),
}

#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwapState {
    Pending = 0,
    Executed = 1,
    Cancelled = 2,
}

impl core::fmt::Display for SwapState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            SwapState::Pending => "pending",
            SwapState::Executed => "executed",
            SwapState::Cancelled => "cancelled",
        })
    }
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BasketLeg {
    pub token: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SwapInfo {
    pub id: u32,
    pub party_a: Address,
    pub token_a: Address,
    pub amount_a: i128,
    pub token_b: Address,
    pub amount_b: i128,
    pub expires_at: u32,
    pub state: SwapState,
    pub filled_amount: i128,
    pub allow_partial: bool,
    pub escrowed: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct BasketSwapInfo {
    pub id: u32,
    pub party_a: Address,
    pub offers: soroban_sdk::Vec<BasketLeg>,
    pub demands: soroban_sdk::Vec<BasketLeg>,
    pub expires_at: u32,
    pub state: SwapState,
}