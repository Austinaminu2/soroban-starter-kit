use soroban_sdk::{Address, Env, Symbol, Vec};

pub fn initialized(env: &Env, admin: &Address) {
    env.events().publish((Symbol::new(env, "initialized"),), admin.clone());
}

pub fn voter_registered(env: &Env, voter: &Address) {
    env.events()
        .publish((Symbol::new(env, "voter_registered"),), voter.clone());
}

pub fn voter_deregistered(env: &Env, voter: &Address) {
    env.events()
        .publish((Symbol::new(env, "voter_deregistered"),), voter.clone());
}

pub fn voted(env: &Env, voter: &Address, choice: u32) {
    env.events()
        .publish((Symbol::new(env, "voted"),), (voter.clone(), choice));
}

/// Emitted by `tally` (binary ballot, backward compat).
pub fn tally_result(env: &Env, yes: i128, no: i128) {
    env.events()
        .publish((Symbol::new(env, "tally_result"),), (yes, no));
}

/// Emitted by `tally_all` (multi-choice ballot, #788).
/// `counts` contains per-choice vote tallies in declaration order.
pub fn tally_all_result(env: &Env, counts: &Vec<i128>) {
    env.events()
        .publish((Symbol::new(env, "tally_all_result"),), counts.clone());
}
