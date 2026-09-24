use soroban_sdk::{Address, Env};

use crate::errors::EscrowError;
use crate::events;
use crate::lifecycle::{extend_ttl, get_required, refund_to_buyer, release_to_seller};
use crate::storage::{DataKey, EscrowState};

use DataKey::*;

pub fn raise_dispute(env: Env, caller: Address) -> Result<(), EscrowError> {
    #[cfg(feature = "pausable")]
    crate::EscrowContract::require_not_paused(&env)?;

    let buyer: Address = get_required(&env, &Buyer)?;
    let seller: Address = get_required(&env, &Seller)?;

    if caller != buyer && caller != seller {
        return Err(EscrowError::NotAuthorized);
    }
    caller.require_auth();

    let state: EscrowState = get_required(&env, &State)?;
    if !matches!(state, EscrowState::Funded | EscrowState::Delivered) {
        return Err(EscrowError::InvalidState);
    }

    env.storage().instance().set(&State, &EscrowState::Disputed);
    // Record when the dispute was raised for timeout tracking (issue #709)
    env.storage()
        .instance()
        .set(&DisputeRaisedAt, &env.ledger().sequence());
    extend_ttl(&env);

    events::dispute_raised(&env, &caller);

    Ok(())
}

pub fn claim_dispute_timeout(env: Env) -> Result<(), EscrowError> {
    #[cfg(feature = "pausable")]
    crate::EscrowContract::require_not_paused(&env)?;

    let buyer: Address = get_required(&env, &Buyer)?;
    buyer.require_auth();

    let state: EscrowState = get_required(&env, &State)?;
    if state != EscrowState::Disputed {
        return Err(EscrowError::InvalidState);
    }

    let timeout: u32 = env
        .storage()
        .instance()
        .get(&DisputeTimeoutLedgers)
        .unwrap_or(0);
    if timeout == 0 {
        return Err(EscrowError::NoDisputeTimeout);
    }

    let raised_at: u32 = get_required(&env, &DisputeRaisedAt)?;
    let elapsed = env.ledger().sequence().saturating_sub(raised_at);
    if elapsed < timeout {
        return Err(EscrowError::DisputeTimeoutNotReached);
    }

    // Auto-resolve in buyer's favour
    env.storage().instance().set(&State, &EscrowState::Funded);
    env.storage().instance().remove(&DisputeRaisedAt);
    refund_to_buyer(env)
}

pub fn resolve_dispute(
    env: Env,
    caller: Address,
    release_to_seller_flag: bool,
) -> Result<(), EscrowError> {
    let state: EscrowState = get_required(&env, &State)?;
    if state != EscrowState::Disputed {
        return Err(EscrowError::InvalidState);
    }

    let arbiters_opt: Option<soroban_sdk::Vec<Address>> =
        env.storage().instance().get(&DataKey::Arbiters);

    if let Some(arbiters) = arbiters_opt {
        let required_sigs: u32 = env
            .storage()
            .instance()
            .get(&DataKey::RequiredSignatures)
            .unwrap_or(1);
        resolve_multisig(env, caller, arbiters, required_sigs, release_to_seller_flag)
    } else {
        resolve_single(env, release_to_seller_flag)
    }
}

fn resolve_single(env: Env, release_to_seller_flag: bool) -> Result<(), EscrowError> {
    let arbiter: Address = env
        .storage()
        .instance()
        .get(&Arbiter)
        .ok_or(EscrowError::NotInitialized)?;
    arbiter.require_auth();

    if release_to_seller_flag {
        env.storage()
            .instance()
            .set(&State, &EscrowState::Delivered);
        release_to_seller(env)
    } else {
        env.storage().instance().set(&State, &EscrowState::Funded);
        refund_to_buyer(env)
    }
}

fn resolve_multisig(
    env: Env,
    caller: Address,
    arbiters: soroban_sdk::Vec<Address>,
    required_sigs: u32,
    release_to_seller_flag: bool,
) -> Result<(), EscrowError> {
    let mut is_valid_arbiter = false;
    for arbiter in arbiters.iter() {
        if arbiter == caller {
            is_valid_arbiter = true;
            break;
        }
    }

    if !is_valid_arbiter {
        return Err(EscrowError::NotAuthorized);
    }

    caller.require_auth();

    // Separate vote tracking by direction
    let votes_key = if release_to_seller_flag {
        DataKey::ArbiterVotesRelease
    } else {
        DataKey::ArbiterVotesRefund
    };

    let mut votes: soroban_sdk::Vec<Address> = env
        .storage()
        .instance()
        .get(&votes_key)
        .unwrap_or_else(|| soroban_sdk::Vec::new(&env));

    // Check if arbiter already voted for this direction
    if votes.iter().any(|v| v == caller) {
        // Already voted for this direction, no-op
        return Ok(());
    }

    // Check if arbiter already voted for the opposite direction
    let opposite_key = if release_to_seller_flag {
        DataKey::ArbiterVotesRefund
    } else {
        DataKey::ArbiterVotesRelease
    };

    let opposite_votes: soroban_sdk::Vec<Address> = env
        .storage()
        .instance()
        .get(&opposite_key)
        .unwrap_or_else(|| soroban_sdk::Vec::new(&env));

    if opposite_votes.iter().any(|v| v == caller) {
        // Arbiter already voted for the opposite direction - reject conflicting vote
        return Err(EscrowError::NotAuthorized);
    }

    // Record the vote for this direction
    votes.push_back(caller.clone());
    env.storage().instance().set(&votes_key, &votes);

    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
    if votes.len() as u32 >= required_sigs {
        // Threshold reached for this direction - resolve the dispute
        env.storage()
            .instance()
            .remove(&DataKey::ArbiterVotesRelease);
        env.storage()
            .instance()
            .remove(&DataKey::ArbiterVotesRefund);
        extend_ttl(&env);
        if release_to_seller_flag {
            env.storage()
                .instance()
                .set(&State, &EscrowState::Delivered);
            release_to_seller(env)
        } else {
            env.storage().instance().set(&State, &EscrowState::Funded);
            refund_to_buyer(env)
        }
    } else {
        extend_ttl(&env);
        Ok(())
    }
}
