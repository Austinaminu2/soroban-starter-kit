#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use soroban_sdk::{
    Address, Env,
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
};

use crate::{VestingContract, VestingContractClient, VestingError};

// ── helpers ──────────────────────────────────────────────────────────────────

pub(crate) fn setup_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.sequence_number = 100);
    env
}

pub(crate) fn make_token(env: &Env, mint_to: &Address, amount: i128) -> Address {
    let sac = env.register_stellar_asset_contract_v2(Address::generate(env));
    let addr = sac.address();
    StellarAssetClient::new(env, &addr).mint(mint_to, &amount);
    addr
}

pub(crate) fn setup(
    env: &Env,
) -> (
    VestingContractClient,
    Address,
    Address,
    Address,
    u32,
    u32,
    i128,
) {
    let admin = Address::generate(env);
    let beneficiary = Address::generate(env);
    let amount = 1_000i128;
    let token = make_token(env, &admin, amount * 2); // Mint extra to allow for potential multiple schedules
    let cliff = env.ledger().sequence() + 10;
    let end = cliff + 100;
    let addr = env.register_contract(None, VestingContract);
    let client = VestingContractClient::new(env, &addr);
    client.initialize(&admin, &token);
    client.create_schedule(&beneficiary, &cliff, &end, &amount);
    (client, admin, beneficiary, token, cliff, end, amount)
}

// ── unit tests ────────────────────────────────────────────────────────────────

#[test]
fn test_initialize_stores_info() {
    let env = setup_env();
    let (client, _admin, beneficiary, _token, cliff, end, amount) = setup(&env);
    let info = client.get_info(&beneficiary).unwrap();
    assert_eq!(info.amount, amount);
    assert_eq!(info.cliff_ledger, cliff);
    assert_eq!(info.end_ledger, end);
    assert_eq!(info.claimed, 0);
    assert!(!info.revoked);
}

#[test]
fn test_initialize_twice_fails() {
    let env = setup_env();
    let (client, admin, _beneficiary, token, ..) = setup(&env);
    let result = client.try_initialize(&admin, &token);
    assert_eq!(result, Err(Ok(VestingError::AlreadyInitialized)));
}

#[test]
fn test_create_schedule_zero_amount_fails() {
    let env = setup_env();
    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = make_token(&env, &admin, 0);
    let addr = env.register_contract(None, VestingContract);
    let client = VestingContractClient::new(&env, &addr);
    client.initialize(&admin, &token);
    let result = client.try_create_schedule(&beneficiary, &110u32, &200u32, &0i128);
    assert_eq!(result, Err(Ok(VestingError::InvalidAmount)));
}

#[test]
fn test_create_schedule_invalid_schedule_fails() {
    let env = setup_env();
    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = make_token(&env, &admin, 1000);
    let addr = env.register_contract(None, VestingContract);
    let client = VestingContractClient::new(&env, &addr);
    client.initialize(&admin, &token);
    // cliff >= end
    let result = client.try_create_schedule(&beneficiary, &200u32, &150u32, &1000i128);
    assert_eq!(result, Err(Ok(VestingError::InvalidSchedule)));
}

#[test]
fn test_claim_before_cliff_fails() {
    let env = setup_env();
    let (client, _admin, beneficiary, ..) = setup(&env);
    let result = client.try_claim(&beneficiary);
    assert_eq!(result, Err(Ok(VestingError::NothingToClaim)));
}

#[test]
fn test_claim_at_cliff_returns_zero() {
    let env = setup_env();
    let (client, _admin, beneficiary, _token, cliff, _end, _amount) = setup(&env);
    env.ledger().with_mut(|l| l.sequence_number = cliff);
    let result = client.try_claim(&beneficiary);
    assert_eq!(result, Err(Ok(VestingError::NothingToClaim)));
}

#[test]
fn test_claim_halfway_through_vesting() {
    let env = setup_env();
    let (client, _admin, beneficiary, _token, cliff, end, amount) = setup(&env);
    let mid = cliff + (end - cliff) / 2;
    env.ledger().with_mut(|l| l.sequence_number = mid);
    let claimed = client.claim(&beneficiary);
    assert!(claimed > 0 && claimed <= amount / 2 + 1);
}

#[test]
fn test_claim_after_end_returns_full_amount() {
    let env = setup_env();
    let (client, _admin, beneficiary, token, _cliff, end, amount) = setup(&env);
    env.ledger().with_mut(|l| l.sequence_number = end + 1);
    let claimed = client.claim(&beneficiary);
    assert_eq!(claimed, amount);
    let token_client = soroban_sdk::token::Client::new(&env, &token);
    assert_eq!(token_client.balance(&beneficiary), amount);
}

#[test]
fn test_double_claim_second_returns_nothing() {
    let env = setup_env();
    let (client, _admin, beneficiary, _token, _cliff, end, _amount) = setup(&env);
    env.ledger().with_mut(|l| l.sequence_number = end + 1);
    client.claim(&beneficiary);
    let result = client.try_claim(&beneficiary);
    assert_eq!(result, Err(Ok(VestingError::NothingToClaim)));
}

#[test]
fn test_revoke_before_cliff_returns_all() {
    let env = setup_env();
    let (client, admin, beneficiary, token, _cliff, _end, amount) = setup(&env);
    let token_client = soroban_sdk::token::Client::new(&env, &token);
    let admin_balance_before = token_client.balance(&admin);
    let returned = client.revoke(&beneficiary);
    assert_eq!(returned, amount);
    // Before the cliff, nothing is vested — the schedule's full `amount` comes back.
    assert_eq!(token_client.balance(&admin), admin_balance_before + amount);
}

#[test]
fn test_revoke_after_end_returns_nothing() {
    let env = setup_env();
    let (client, _admin, beneficiary, _token, _cliff, end, _amount) = setup(&env);
    env.ledger().with_mut(|l| l.sequence_number = end + 1);
    let returned = client.revoke(&beneficiary);
    assert_eq!(returned, 0);
}

#[test]
fn test_revoke_midway_returns_unvested_portion() {
    let env = setup_env();
    let (client, admin, beneficiary, token, cliff, end, amount) = setup(&env);
    let token_client = soroban_sdk::token::Client::new(&env, &token);
    let admin_balance_before = token_client.balance(&admin);
    let mid = cliff + (end - cliff) / 2;
    env.ledger().with_mut(|l| l.sequence_number = mid);
    let returned = client.revoke(&beneficiary);
    assert!(returned > 0 && returned < amount);
    assert_eq!(token_client.balance(&admin), admin_balance_before + returned);
}

#[test]
fn test_claim_after_revoke_gets_vested_portion() {
    let env = setup_env();
    let (client, _admin, beneficiary, _token, cliff, end, amount) = setup(&env);
    let mid = cliff + (end - cliff) / 2;
    env.ledger().with_mut(|l| l.sequence_number = mid);
    let returned = client.revoke(&beneficiary);
    let claimed = client.claim(&beneficiary);
    assert_eq!(claimed + returned, amount);
}

#[test]
fn test_revoke_twice_fails() {
    let env = setup_env();
    let (client, _admin, beneficiary, ..) = setup(&env);
    client.revoke(&beneficiary);
    let result = client.try_revoke(&beneficiary);
    assert_eq!(result, Err(Ok(VestingError::AlreadyRevoked)));
}

#[test]
fn test_claim_after_full_revoke_fails() {
    let env = setup_env();
    let (client, _admin, beneficiary, ..) = setup(&env);
    // revoke before cliff — nothing vested, amount capped to 0
    client.revoke(&beneficiary);
    let result = client.try_claim(&beneficiary);
    assert_eq!(result, Err(Ok(VestingError::NothingToClaim)));
}

#[test]
fn test_get_info_uninitialized_returns_none() {
    let env = setup_env();
    let addr = env.register_contract(None, VestingContract);
    let client = VestingContractClient::new(&env, &addr);
    let beneficiary = Address::generate(&env);
    assert_eq!(client.get_info(&beneficiary), None);
}

#[test]
fn test_claimable_before_cliff_is_zero() {
    let env = setup_env();
    let (client, _admin, beneficiary, ..) = setup(&env);
    let result = client.try_claim(&beneficiary);
    assert_eq!(result, Err(Ok(VestingError::NothingToClaim)));
}

// ── admin_release tests ───────────────────────────────────────────────────────

#[test]
fn test_admin_release_before_cliff_unlocks_all() {
    let env = setup_env();
    let (client, _admin, beneficiary, token, _cliff, _end, amount) = setup(&env);
    let token_client = soroban_sdk::token::Client::new(&env, &token);
    // Still before the cliff.
    let released = client.admin_release(&beneficiary);
    assert_eq!(released, amount);
    assert_eq!(token_client.balance(&beneficiary), amount);
    let info = client.get_info(&beneficiary).unwrap();
    assert_eq!(info.claimed, amount);
}

#[test]
fn test_admin_release_after_cliff_unlocks_remaining() {
    let env = setup_env();
    let (client, _admin, beneficiary, token, cliff, end, amount) = setup(&env);
    let token_client = soroban_sdk::token::Client::new(&env, &token);
    // Move past the cliff but before the end of the schedule.
    let mid = cliff + (end - cliff) / 2;
    env.ledger().with_mut(|l| l.sequence_number = mid);
    let released = client.admin_release(&beneficiary);
    assert_eq!(released, amount);
    assert_eq!(token_client.balance(&beneficiary), amount);
    let info = client.get_info(&beneficiary).unwrap();
    assert_eq!(info.claimed, amount);
}

#[test]
fn test_admin_release_after_partial_claim_releases_remainder() {
    let env = setup_env();
    let (client, _admin, beneficiary, token, cliff, end, amount) = setup(&env);
    let token_client = soroban_sdk::token::Client::new(&env, &token);
    let mid = cliff + (end - cliff) / 2;
    env.ledger().with_mut(|l| l.sequence_number = mid);
    let claimed = client.claim(&beneficiary);
    let released = client.admin_release(&beneficiary);
    assert_eq!(claimed + released, amount);
    assert_eq!(token_client.balance(&beneficiary), amount);
}

#[test]
fn test_admin_release_twice_second_returns_nothing() {
    let env = setup_env();
    let (client, _admin, beneficiary, ..) = setup(&env);
    client.admin_release(&beneficiary);
    let result = client.try_admin_release(&beneficiary);
    assert_eq!(result, Err(Ok(VestingError::NothingToClaim)));
}
