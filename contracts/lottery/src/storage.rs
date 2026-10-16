// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, BytesN, Vec, contracttype};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Token,
    TicketPrice,
    Participants,
    State,
    Commit,
    Winner,
    Winners,
    RevealDeadline,
    WinnerCount,
    PrizeSplits,
    MaxTicketsPerAddress,
    TicketCount(Address),
    /// Fee in basis points (0–9999) deducted from the prize pool before winner payouts.
    FeeBps,
    /// Recipient address for the fee amount.
    FeeRecipient,
}

#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LotteryState {
    Open = 0,
    Committed = 1,
    Drawn = 2,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct LotteryInfo {
    pub admin: Address,
    pub token: Address,
    pub ticket_price: i128,
    pub state: LotteryState,
    pub participants: Vec<Address>,
}

/// Commit stored on-chain: hash(secret || salt) where both are 32-byte values.
#[contracttype]
#[derive(Clone)]
pub struct Commit {
    pub hash: BytesN<32>,
}
