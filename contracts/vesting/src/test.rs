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
    assert_eq!(client.claimable(&beneficiary), 0);
}

#[test]
fn test_claimable_after_end_is_full_amount() {
    let env = setup_env();
    let (client, _admin, beneficiary, _token, _cliff, end, amount) = setup(&env);
    env.ledger().with_mut(|l| l.sequence_number = end + 1);
    assert_eq!(client.claimable(&beneficiary), amount);
}

#[test]
fn test_multiple_beneficiaries_independent_schedules() {
    let env = setup_env();
    let admin = Address::generate(&env);
    let beneficiary1 = Address::generate(&env);
    let beneficiary2 = Address::generate(&env);
    let amount1 = 1000i128;
    let amount2 = 2000i128;
    let total_amount = amount1 + amount2;
    
    // Mint enough tokens for both beneficiaries
    let token = make_token(&env, &admin, total_amount);
    
    // Initialize the contract once
    let addr = env.register_contract(None, VestingContract);
    let client = VestingContractClient::new(&env, &addr);
    client.initialize(&admin, &token);
    
    // Create different schedules for each beneficiary
    let cliff1 = env.ledger().sequence() + 10;
    let end1 = cliff1 + 100;
    client.create_schedule(&beneficiary1, &cliff1, &end1, &amount1);
    
    // cliff2 must land after `mid1` below (mid-way through beneficiary1's
    // schedule) for the "before beneficiary2's cliff" assertions to hold.
    let cliff2 = env.ledger().sequence() + 170; // Different cliff
    let end2 = cliff2 + 200; // Different end
    client.create_schedule(&beneficiary2, &cliff2, &end2, &amount2);
    
    // Verify both schedules exist with correct parameters
    let info1 = client.get_info(&beneficiary1).unwrap();
    assert_eq!(info1.amount, amount1);
    assert_eq!(info1.cliff_ledger, cliff1);
    assert_eq!(info1.end_ledger, end1);
    assert_eq!(info1.claimed, 0);
    assert!(!info1.revoked);
    
    let info2 = client.get_info(&beneficiary2).unwrap();
    assert_eq!(info2.amount, amount2);
    assert_eq!(info2.cliff_ledger, cliff2);
    assert_eq!(info2.end_ledger, end2);
    assert_eq!(info2.claimed, 0);
    assert!(!info2.revoked);
    
    // Move ledger to mid-way through beneficiary1's schedule, but before beneficiary2's cliff
    let mid1 = cliff1 + (end1 - cliff1) / 2;
    env.ledger().with_mut(|l| l.sequence_number = mid1);
    
    // Beneficiary1 should have claimable tokens, beneficiary2 should have 0
    assert!(client.claimable(&beneficiary1) > 0);
    assert_eq!(client.claimable(&beneficiary2), 0);
    
    // Beneficiary1 claims their tokens
    let claimed1 = client.claim(&beneficiary1);
    assert!(claimed1 > 0 && claimed1 < amount1);
    
    // Verify beneficiary2's schedule is completely unaffected
    let info2_after = client.get_info(&beneficiary2).unwrap();
    assert_eq!(info2_after.claimed, 0);
    assert_eq!(client.claimable(&beneficiary2), 0);
    
    // Revoke beneficiary2's schedule before their cliff
    let returned2 = client.revoke(&beneficiary2);
    assert_eq!(returned2, amount2); // All tokens returned to admin

    // Advance to the end of beneficiary1's schedule so more has vested since
    // their first claim.
    env.ledger().with_mut(|l| l.sequence_number = end1);

    // Beneficiary1 can still claim their remaining tokens
    let remaining1 = client.claimable(&beneficiary1);
    assert!(remaining1 > 0);
    let claimed1_again = client.claim(&beneficiary1);
    assert_eq!(claimed1_again, remaining1);
    
    // Verify both schedules are properly updated independently
    let info1_final = client.get_info(&beneficiary1).unwrap();
    assert_eq!(info1_final.claimed, amount1);
    assert!(!info1_final.revoked);
    
    let info2_final = client.get_info(&beneficiary2).unwrap();
    assert_eq!(info2_final.claimed, 0);
    assert!(info2_final.revoked);
    assert_eq!(info2_final.amount, 0); // amount was capped to 0 at revocation
}

// ── property tests ────────────────────────────────────────────────────────────

use proptest::prelude::*;

fn prop_setup(
    env: &Env,
    amount: i128,
) -> (VestingContractClient, Address, Address, Address, u32, u32) {
    let admin = Address::generate(env);
    let beneficiary = Address::generate(env);
    let token = make_token(env, &admin, amount * 2);
    let cliff = env.ledger().sequence() + 10;
    let end = cliff + 100;
    let addr = env.register_contract(None, VestingContract);
    let client = VestingContractClient::new(env, &addr);
    client.initialize(&admin, &token);
    client.create_schedule(&beneficiary, &cliff, &end, &amount);
    (client, admin, beneficiary, token, cliff, end)
}

proptest! {
    #[test]
    fn prop_initialize_stores_amount(amount in 1i128..=1_000_000i128) {
        let env = setup_env();
        let (client, _admin, beneficiary, ..) = prop_setup(&env, amount);
        let info = client.get_info(&beneficiary).unwrap();
        assert_eq!(info.amount, amount);
        assert_eq!(info.claimed, 0);
        assert!(!info.revoked);
    }

    #[test]
    fn prop_claim_after_end_yields_full(amount in 1i128..=1_000_000i128) {
        let env = setup_env();
        let (client, _admin, beneficiary, token, _cliff, end) = prop_setup(&env, amount);
        env.ledger().with_mut(|l| l.sequence_number = end + 1);
        let claimed = client.claim(&beneficiary);
        assert_eq!(claimed, amount);
        let token_client = soroban_sdk::token::Client::new(&env, &token);
        assert_eq!(token_client.balance(&beneficiary), amount);
    }

    #[test]
    fn prop_revoke_before_cliff_returns_all(amount in 1i128..=1_000_000i128) {
        let env = setup_env();
        let (client, admin, beneficiary, token, _cliff, _end) = prop_setup(&env, amount);
        let token_client = soroban_sdk::token::Client::new(&env, &token);
        let admin_balance_before = token_client.balance(&admin);
        let returned = client.revoke(&beneficiary);
        assert_eq!(returned, amount);
        assert_eq!(token_client.balance(&admin), admin_balance_before + amount);
    }

    #[test]
    fn prop_revoke_plus_claim_equals_total(
        amount in 2i128..=1_000_000i128,
        pct in 0u32..=100u32,
    ) {
        let env = setup_env();
        let (client, _admin, beneficiary, _token, cliff, end) = prop_setup(&env, amount);
        let ledger = cliff + (end - cliff) * pct / 100;
        env.ledger().with_mut(|l| l.sequence_number = ledger);
        let returned = client.revoke(&beneficiary);
        let claimed = client.try_claim(&beneficiary).unwrap_or(Ok(0)).unwrap_or(0);
        assert_eq!(returned + claimed, amount);
    }

    #[test]
    fn prop_claimable_monotone(
        amount in 1i128..=1_000_000i128,
        t1_pct in 0u32..=100u32,
        t2_pct in 0u32..=100u32,
    ) {
        let env = setup_env();
        let (client, _admin, beneficiary, ..) = prop_setup(&env, amount);
        let info = client.get_info(&beneficiary).unwrap();
        let cliff = info.cliff_ledger;
        let end = info.end_ledger;

        let l1 = cliff + (end - cliff) * t1_pct / 100;
        let l2 = cliff + (end - cliff) * t2_pct / 100;

        env.ledger().with_mut(|l| l.sequence_number = l1);
        let c1 = client.claimable(&beneficiary);
        env.ledger().with_mut(|l| l.sequence_number = l2);
        let c2 = client.claimable(&beneficiary);

        if l2 >= l1 {
            assert!(c2 >= c1);
        } else {
            assert!(c2 <= c1);
        }
    }
}

// ── #712 admin_release tests ─────────────────────────────────────────────────

#[test]
fn test_admin_release_before_cliff_sends_all_to_beneficiary() {
    let env = setup_env();
    let (client, _admin, beneficiary, token, _cliff, _end, amount) = setup(&env);
    // Still before cliff (ledger = 100, cliff = 110)
    let released = client.admin_release(&beneficiary);
    assert_eq!(released, amount);
    let token_client = soroban_sdk::token::Client::new(&env, &token);
    assert_eq!(token_client.balance(&beneficiary), amount);
}

#[test]
fn test_admin_release_marks_revoked() {
    let env = setup_env();
    let (client, _, beneficiary, ..) = setup(&env);
    let (client, _admin, beneficiary, ..) = setup(&env);
    client.admin_release(&beneficiary);
    let info = client.get_info(&beneficiary).unwrap();
    assert!(info.revoked);
}

#[test]
fn test_admin_release_after_cliff_fails() {
    let env = setup_env();
    let (client, _admin, beneficiary, _token, cliff, _end, _amount) = setup(&env);
    // Advance past cliff
    env.ledger().with_mut(|l| l.sequence_number = cliff);
    let result = client.try_admin_release(&beneficiary);
    assert_eq!(result, Err(Ok(VestingError::CliffAlreadyPassed)));
}

#[test]
fn test_admin_release_twice_fails() {
    let env = setup_env();
    let (client, _, beneficiary, ..) = setup(&env);
    let (client, _admin, beneficiary, ..) = setup(&env);
    client.admin_release(&beneficiary);
    let result = client.try_admin_release(&beneficiary);
    assert_eq!(result, Err(Ok(VestingError::AlreadyRevoked)));
}

#[test]
fn test_admin_release_emits_event() {
    let env = setup_env();
    let (client, admin, beneficiary, _token, _cliff, _end, amount) = setup(&env);
    client.admin_release(&beneficiary);
    use soroban_sdk::{IntoVal, Symbol, testutils::Events as _};
    use soroban_sdk::{FromVal, IntoVal, Symbol, testutils::Events as _};
    let all = env.events().all();
    let found = all.iter().any(|(_, topics, data)| {
        topics == (Symbol::new(&env, "admin_released"), admin.clone()).into_val(&env)
            && i128::from_val(&env, &data) == amount
    });
    assert!(found, "admin_released event not emitted");
}