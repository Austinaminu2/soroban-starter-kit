use soroban_sdk::{Address, Env, Symbol, Vec};

pub fn initialized(env: &Env, threshold: u32, signer_count: u32) {
    env.events()
        .publish((Symbol::new(env, "initialized"), threshold), signer_count);
}

pub fn signer_added(env: &Env, signer: &Address, threshold: u32) {
    env.events()
        .publish((Symbol::new(env, "added"), signer.clone()), threshold);
}

pub fn signer_removed(env: &Env, signer: &Address, threshold: u32) {
    env.events()
        .publish((Symbol::new(env, "removed"), signer.clone()), threshold);
}

pub fn transaction_proposed(env: &Env, tx_id: u64, proposer: &Address) {
    env.events()
        .publish((Symbol::new(env, "proposed"), proposer.clone()), tx_id);
}

pub fn transaction_signed(env: &Env, tx_id: u64, signer: &Address, signature_count: u32) {
    env.events().publish(
        (Symbol::new(env, "signed"), signer.clone(), tx_id),
        signature_count,
    );
}

pub fn transaction_executed(env: &Env, tx_id: u64) {
    env.events()
        .publish((Symbol::new(env, "executed"), tx_id), ());
}

pub fn proposal_expired(env: &Env, tx_id: u64) {
    env.events()
        .publish((Symbol::new(env, "expired"), tx_id), ());
}

/// Emitted after a `execute_batch` call completes.
///
/// `executed_ids` — proposals that were successfully executed.
/// `skipped_ids`  — proposals that were skipped (already executed, expired,
///                  threshold not met, or not found) along with their error codes.
pub fn batch_executed(env: &Env, executed_ids: &Vec<u64>, skipped_count: u32) {
    env.events().publish(
        (Symbol::new(env, "batch_executed"),),
        (executed_ids.clone(), skipped_count),
    );
}

pub fn transaction_queued(env: &Env, tx_id: u64, executable_at: u32) {
    env.events()
        .publish((Symbol::new(env, "queued"), tx_id), executable_at);
}

pub fn transaction_cancelled(env: &Env, tx_id: u64, canceller: &Address) {
    env.events()
        .publish((Symbol::new(env, "cancelled"), tx_id), canceller.clone());
}

pub fn timelock_updated(env: &Env, delay: u32) {
    env.events()
        .publish((Symbol::new(env, "timelock_set"),), delay);
}

pub fn spending_limit_updated(env: &Env, operator: &Address, token: &Address, daily_limit: i128) {
    env.events().publish(
        (Symbol::new(env, "spend_limit_set"), operator.clone()),
        (token.clone(), daily_limit),
    );
}

pub fn allowance_spent(env: &Env, operator: &Address, to: &Address, amount: i128, spent: i128) {
    env.events().publish(
        (Symbol::new(env, "allowance_spent"), operator.clone(), to.clone()),
        (amount, spent),
    );
}
