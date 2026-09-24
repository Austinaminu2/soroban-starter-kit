use soroban_sdk::{Address, Env, Symbol, Val};

pub fn initialized(env: &Env, admin: &Address, token: &Address, quorum: i128) {
    env.events().publish(
        (
            Symbol::new(env, "initialized"),
            admin.clone(),
            token.clone(),
        ),
        quorum,
    );
}

pub fn proposal_created(env: &Env, proposer: &Address, proposal_id: u32) {
    env.events()
        .publish((Symbol::new(env, "created"), proposer.clone()), proposal_id);
}

pub fn voted(env: &Env, voter: &Address, proposal_id: u32, support: bool, weight: i128) {
    env.events().publish(
        (Symbol::new(env, "voted"), voter.clone()),
        (proposal_id, support, weight),
    );
}

pub fn proposal_executed(env: &Env, proposal_id: u32) {
    env.events()
        .publish((Symbol::new(env, "executed"),), proposal_id);
}

/// Emitted after a proposal's action payload is dispatched (issue #1108).
///
/// `return_val` is the `Val` returned by the invoked contract function.
pub fn proposal_action_executed(env: &Env, proposal_id: u32, return_val: Val) {
    env.events().publish(
        (Symbol::new(env, "action_executed"),),
        (proposal_id, return_val),
    );
}

pub fn proposal_cancelled(env: &Env, admin: &Address, proposal_id: u32) {
    env.events()
        .publish((Symbol::new(env, "cancelled"), admin.clone()), proposal_id);
}

/// Emitted when a proposal bond is slashed to the treasury (issue #1106).
pub fn bond_slashed(env: &Env, proposer: &Address, proposal_id: u32, amount: i128) {
    env.events().publish(
        (Symbol::new(env, "bond_slashed"), proposer.clone()),
        (proposal_id, amount),
    );
}

/// Emitted when a proposal bond is refunded to the proposer (issue #1106).
pub fn bond_refunded(env: &Env, proposer: &Address, proposal_id: u32, amount: i128) {
    env.events().publish(
        (Symbol::new(env, "bond_refunded"), proposer.clone()),
        (proposal_id, amount),
    );
}

/// Emitted when the adaptive quorum EMA is updated (issue #1107).
pub fn quorum_updated(env: &Env, new_quorum_bps: u32) {
    env.events()
        .publish((Symbol::new(env, "quorum_updated"),), new_quorum_bps);
}
