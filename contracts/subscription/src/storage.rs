// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, contracttype, Symbol};

/// Top-level storage keys used by [`SubscriptionContract`](crate::SubscriptionContract).
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// The service provider's [`Address`] (instance storage).
    Provider,
    /// The payment token contract [`Address`] (instance storage).
    Token,
    /// Per-subscriber [`SubscriptionInfo`] (persistent storage).
    Subscription(Address),
    /// Plan details for a named plan (persistent storage).
    Plan(Symbol),
}

/// Plan configuration that can be registered by the admin.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Unique identifier for the plan.
    pub plan_id: Symbol,
    /// Amount of tokens charged per interval.
    pub amount: i128,
    /// Number of ledgers between each charge.
    pub interval_ledgers: u32,
    /// Whether the plan is active and can be subscribed to.
    pub active: bool,
}

/// Subscription configuration and state for a single subscriber.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct SubscriptionInfo {
    /// The plan ID that this subscriber is subscribed to.
    pub plan_id: Symbol,
    /// Amount of tokens charged per interval (copied from plan at subscription time).
    pub amount: i128,
    /// Number of ledgers between each charge (copied from plan at subscription time).
    pub interval_ledgers: u32,
    /// Number of ledgers in the trial period (if any).
    pub trial_ledgers: u32,
    /// Whether the trial period has been completed (first charge processed).
    pub trial_completed: bool,
    /// Start ledger of the most recently paid billing period (or subscription start
    /// while in trial). The next period falls due `interval_ledgers` after this.
    pub last_charged_ledger: u32,
    /// Whether the subscription is currently active.
    pub active: bool,
}