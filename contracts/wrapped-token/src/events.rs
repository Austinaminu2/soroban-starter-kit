use soroban_sdk::{Address, Env};

pub fn initialized(env: &Env, admin: &Address, token: &Address) {
    let topics = (Symbol::new(env, "initialized"),);
    env.events().publish(topics, (admin.clone(), token.clone()));
}

pub fn wrapped(env: &Env, user: &Address, amount: i128, total: i128) {
    let topics = (Symbol::new(env, "wrapped"),);
    env.events().publish(topics, (user.clone(), amount, total));
}

pub fn unwrapped(env: &Env, user: &Address, amount: i128, total: i128) {
    let topics = (Symbol::new(env, "unwrapped"),);
    env.events().publish(topics, (user.clone(), amount, total));
}

/// Emitted when the contract is paused. Only used when the `pausable` feature is enabled.
#[cfg(feature = "pausable")]
pub fn paused(env: &Env, admin: &Address) {
    env.events()
        .publish((Symbol::new(env, "paused"), admin.clone()), ());
}

/// Emitted when the contract is unpaused. Only used when the `pausable` feature is enabled.
#[cfg(feature = "pausable")]
pub fn unpaused(env: &Env, admin: &Address) {
    env.events()
        .publish((Symbol::new(env, "unpaused"), admin.clone()), ());
}

/// Emitted when the `wrap` pause flag changes. Only used when the `pausable` feature is enabled.
#[cfg(feature = "pausable")]
pub fn wrap_paused(env: &Env, admin: &Address, paused: bool) {
    env.events()
        .publish((Symbol::new(env, "wrap_paused"), admin.clone()), paused);
}

/// Emitted when the `unwrap` pause flag changes. Only used when the `pausable` feature is enabled.
#[cfg(feature = "pausable")]
pub fn unwrap_paused(env: &Env, admin: &Address, paused: bool) {
    env.events()
        .publish((Symbol::new(env, "unwrap_paused"), admin.clone()), paused);
}

/// Emitted when a protocol fee is collected. `op` is `wrap` or `unwrap`.
pub fn fee_collected(env: &Env, op: Symbol, payer: &Address, recipient: &Address, fee: i128) {
    let topics = (Symbol::new(env, "fee_collected"), op);
    env.events()
        .publish(topics, (payer.clone(), recipient.clone(), fee));
}

use soroban_sdk::Symbol;
