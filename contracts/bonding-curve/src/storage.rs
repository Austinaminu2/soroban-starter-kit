// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, contracttype};

#[contracttype]
#[derive(Clone, Debug)]
pub enum DataKey {
    Admin,
    Token,
    Reserve,
    Supply,
    Price,
    CurveBalance(Address),
}

/// Price scale factor for fixed-point arithmetic
pub const PRICE_SCALE: i128 = 1_000_000;
