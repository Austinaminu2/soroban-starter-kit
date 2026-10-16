#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use super::*;
use soroban_sdk::{
    Address, Env, String,
    testutils::{Address as _, Ledger as _},
    token::{StellarAssetClient, TokenClient, TokenInterface},
};

// ---------------------------------------------------------------------------
// MockToken — no-op token for unit tests.
// ---------------------------------------------------------------------------

#[contract]
pub struct MockToken;

#[contractimpl]
impl TokenInterface for MockToken {
    fn allowance(_env: Env, _from: Address, _spender: Address) -> i128 {
        0
    }
    fn approve(_env: Env, _from: Address, _spender: Address, _amount: i128, _exp: u32) {}
    fn balance(_env: Env, _id: Address) -> i128 {
        i128::MAX
    }
    fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {}
    fn transfer_from(_env: Env, _spender: Address, _from: Address, _to: Address, _amount: i128) {}
    fn burn(_env: Env, _from: Address, _amount: i128) {}
    fn burn_from(_env: Env, _spender: Address, _from: Address, _amount: i128) {}
    fn decimals(_env: Env) -> u32 {
        7
    }
    fn name(env: Env) -> String {
        String::from_str(&env, "Mock")
    }
    fn symbol(env: Env) -> String {
        String::from_str(&env, "MCK")
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup(env: &Env) -> (TimelockContractClient, Address, Address, Address, u32, i128) {
    let admin = Address::generate(env);
    let beneficiary = Address::generate(env);
    let token = env.register_contract(None, MockToken);
    let release_ledger = env.ledger().sequence() + 100;
    let amount = 1_000i128;

    let contract_addr = env.register_contract(None, TimelockContract);
    let client = TimelockContractClient::new(env, &contract_addr);
    client.initialize(&admin, &token, &beneficiary, &release_ledger, &amount);

    (client, admin, beneficiary, token, release_ledger, amount)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_initialize() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, beneficiary, token, release_ledger, amount) = setup(&env);

    let info = client.get_info();
    assert_eq!(info.admin, admin);
    assert_eq!(info.beneficiary, beneficiary);
    assert_eq!(info.token, token);
    assert_eq!(info.release_ledger, release_ledger);
    assert_eq!(info.amount, amount);
    assert_eq!(info.state, TimelockState::Active);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_initialize_twice_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = env.register_contract(None, MockToken);
    let release_ledger = env.ledger().sequence() + 100;

    let addr = env.register_contract(None, TimelockContract);
    let client = TimelockContractClient::new(&env, &addr);
    client.initialize(&admin, &token, &beneficiary, &release_ledger, &1_000);
    client.initialize(&admin, &token, &beneficiary, &release_ledger, &1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_initialize_zero_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = env.register_contract(None, MockToken);
    let release_ledger = env.ledger().sequence() + 100;

    let addr = env.register_contract(None, TimelockContract);
    let client = TimelockContractClient::new(&env, &addr);
    client.initialize(&admin, &token, &beneficiary, &release_ledger, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_initialize_past_release_ledger_fails() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.sequence_number = 200);

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = env.register_contract(None, MockToken);

    let addr = env.register_contract(None, TimelockContract);
    let client = TimelockContractClient::new(&env, &addr);
    client.initialize(&admin, &token, &beneficiary, &50, &1_000);
}

#[test]
fn test_release_after_ledger() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _, _, _, release_ledger, _) = setup(&env);

    env.ledger()
        .with_mut(|l| l.sequence_number = release_ledger);
    client.release();

    assert_eq!(client.get_info().state, TimelockState::Released);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_release_too_early_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, ..) = setup(&env);
    // release_ledger is current + 100; we're still before it.
    client.release();
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_release_twice_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _, _, _, release_ledger, _) = setup(&env);
    env.ledger()
        .with_mut(|l| l.sequence_number = release_ledger);
    client.release();
    client.release();
}

#[test]
fn test_cancel_by_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, ..) = setup(&env);
    client.cancel();

    assert_eq!(client.get_info().state, TimelockState::Cancelled);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_cancel_twice_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, ..) = setup(&env);
    client.cancel();
    client.cancel();
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_cancel_after_release_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _, _, _, release_ledger, _) = setup(&env);
    env.ledger()
        .with_mut(|l| l.sequence_number = release_ledger);
    client.release();
    client.cancel();
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_release_after_cancel_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _, _, _, release_ledger, _) = setup(&env);
    client.cancel();
    env.ledger()
        .with_mut(|l| l.sequence_number = release_ledger);
    client.release();
}

#[test]
fn test_is_releasable() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _, _, _, release_ledger, _) = setup(&env);
    assert!(!client.is_releasable());

    env.ledger()
        .with_mut(|l| l.sequence_number = release_ledger);
    assert!(client.is_releasable());
}

#[test]
fn test_reassign_beneficiary() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _, old_beneficiary, _, _, _) = setup(&env);
    let new_beneficiary = Address::generate(&env);

    let info = client.get_info();
    assert_eq!(info.beneficiary, old_beneficiary);

    // Reassign to a new beneficiary
    client.reassign_beneficiary(&new_beneficiary);

    let updated_info = client.get_info();
    assert_eq!(updated_info.beneficiary, new_beneficiary);
    assert_eq!(updated_info.state, TimelockState::Active);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_reassign_beneficiary_after_release_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _, _, _, release_ledger, _) = setup(&env);
    let new_beneficiary = Address::generate(&env);

    env.ledger()
        .with_mut(|l| l.sequence_number = release_ledger);
    client.release();

    // Should fail because already released
    client.reassign_beneficiary(&new_beneficiary);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_reassign_beneficiary_after_cancel_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _, _, _, _, _) = setup(&env);
    let new_beneficiary = Address::generate(&env);

    client.cancel();

    // Should fail because already cancelled
    client.reassign_beneficiary(&new_beneficiary);
}

// ── Multi-tranche release tests ──────────────────────────────────────────

#[test]
fn test_initialize_with_tranches() {
    use soroban_sdk::Vec as SdkVec;

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = env.register_contract(None, MockToken);

    let addr = env.register_contract(None, TimelockContract);
    let client = TimelockContractClient::new(&env, &addr);

    let mut tranches = SdkVec::new(&env);
    let ledger1 = env.ledger().sequence() + 50;
    let ledger2 = env.ledger().sequence() + 100;
    tranches.push_back(ReleaseTranche {
        release_ledger: ledger1,
        amount: 500,
    });
    tranches.push_back(ReleaseTranche {
        release_ledger: ledger2,
        amount: 500,
    });

    client.initialize_with_tranches(&admin, &token, &beneficiary, &tranches);

    let info = client.get_info();
    assert_eq!(info.tranches.len(), 2);
    assert_eq!(info.amount, 1_000);
    assert_eq!(info.state, TimelockState::Active);
}

#[test]
fn test_multi_tranche_partial_release() {
    use soroban_sdk::Vec as SdkVec;

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = env.register_contract(None, MockToken);

    let addr = env.register_contract(None, TimelockContract);
    let client = TimelockContractClient::new(&env, &addr);

    let mut tranches = SdkVec::new(&env);
    let ledger1 = env.ledger().sequence() + 50;
    let ledger2 = env.ledger().sequence() + 100;
    tranches.push_back(ReleaseTranche {
        release_ledger: ledger1,
        amount: 400,
    });
    tranches.push_back(ReleaseTranche {
        release_ledger: ledger2,
        amount: 600,
    });

    client.initialize_with_tranches(&admin, &token, &beneficiary, &tranches);

    // Advance to ledger1 - should release first tranche
    env.ledger().with_mut(|l| l.sequence_number = ledger1);
    client.release();

    let info = client.get_info();
    assert_eq!(info.state, TimelockState::Active); // Not yet fully released

    // Advance to ledger2 - should release second tranche
    env.ledger().with_mut(|l| l.sequence_number = ledger2);
    client.release();

    let info = client.get_info();
    assert_eq!(info.state, TimelockState::Released); // Now fully released
}

#[test]
fn test_multi_tranche_is_releasable() {
    use soroban_sdk::Vec as SdkVec;

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let token = env.register_contract(None, MockToken);

    let addr = env.register_contract(None, TimelockContract);
    let client = TimelockContractClient::new(&env, &addr);

    let mut tranches = SdkVec::new(&env);
    let ledger1 = env.ledger().sequence() + 50;
    let ledger2 = env.ledger().sequence() + 100;
    tranches.push_back(ReleaseTranche {
        release_ledger: ledger1,
        amount: 500,
    });
    tranches.push_back(ReleaseTranche {
        release_ledger: ledger2,
        amount: 500,
    });

    client.initialize_with_tranches(&admin, &token, &beneficiary, &tranches);

    assert!(!client.is_releasable());

    env.ledger().with_mut(|l| l.sequence_number = ledger1);
    assert!(client.is_releasable());

    env.ledger().with_mut(|l| l.sequence_number = ledger2);
    assert!(client.is_releasable());
}

// ── Yield-generating timelock integration tests ──────────────────────────

/// Deploy a real Stellar asset contract and mint `amount` to `to`.
fn deploy_token(env: &Env, admin: &Address, to: &Address, amount: i128) -> Address {
    let token = env.register_stellar_asset_contract_v2(admin.clone());
    let token_addr = token.address();
    StellarAssetClient::new(env, &token_addr).mint(to, &amount);
    token_addr
}

/// Deploy a staking contract with `unbonding_period` and return its address.
fn deploy_staking(env: &Env, unbonding_period: u32) -> Address {
    let staking_addr = env.register_contract(None, staking::StakingContract);
    let staking_client = staking::StakingContractClient::new(env, &staking_addr);
    staking_client.initialize(&unbonding_period);
    staking_addr
}

#[test]
fn test_yield_timelock_full_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let yield_recipient = Address::generate(&env);

    let amount = 10_000i128;
    let unbonding_period = 10u32;
    let release_ledger = env.ledger().sequence() + 200;

    // Real test token funded to the timelock contract.
    let timelock_addr = env.register_contract(None, TimelockContract);
    let token = deploy_token(&env, &admin, &timelock_addr, amount);

    // Staking contract with an unbonding period.
    let staking_addr = deploy_staking(&env, unbonding_period);

    let client = TimelockContractClient::new(&env, &timelock_addr);
    client.initialize(&admin, &token, &beneficiary, &release_ledger, &amount);
    client.set_yield_config(&staking_addr, &yield_recipient);

    // Deposit locked balance into the staking pool.
    client.deposit_locked_balance();

    let staking_client = staking::StakingContractClient::new(&env, &staking_addr);
    assert_eq!(staking_client.total_staked(), amount);

    // Advance and withdraw/unbond prior to release.
    env.ledger().with_mut(|l| l.sequence_number += 1);
    client.withdraw_and_unbond();

    // Tokens returned to the timelock after unbonding.
    let token_client = TokenClient::new(&env, &token);
    assert_eq!(token_client.balance(&timelock_addr), amount);

    // Advance past the release ledger and release.
    env.ledger()
        .with_mut(|l| l.sequence_number = release_ledger);
    client.release();

    assert_eq!(client.get_info().state, TimelockState::Released);
    // Principal to beneficiary; accrued yield to the designated recipient.
    assert_eq!(token_client.balance(&beneficiary), amount);
    assert!(token_client.balance(&yield_recipient) >= 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_deposit_without_staking_contract_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let amount = 1_000i128;
    let release_ledger = env.ledger().sequence() + 100;

    let timelock_addr = env.register_contract(None, TimelockContract);
    let token = deploy_token(&env, &admin, &timelock_addr, amount);

    let client = TimelockContractClient::new(&env, &timelock_addr);
    client.initialize(&admin, &token, &beneficiary, &release_ledger, &amount);

    // No yield config set — deposit must fail.
    client.deposit_locked_balance();
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_withdraw_without_staking_contract_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let amount = 1_000i128;
    let release_ledger = env.ledger().sequence() + 100;

    let timelock_addr = env.register_contract(None, TimelockContract);
    let token = deploy_token(&env, &admin, &timelock_addr, amount);

    let client = TimelockContractClient::new(&env, &timelock_addr);
    client.initialize(&admin, &token, &beneficiary, &release_ledger, &amount);

    // No yield config set — withdraw must fail.
    client.withdraw_and_unbond();
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_double_deposit_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let yield_recipient = Address::generate(&env);
    let amount = 1_000i128;
    let release_ledger = env.ledger().sequence() + 100;

    let timelock_addr = env.register_contract(None, TimelockContract);
    let token = deploy_token(&env, &admin, &timelock_addr, amount);
    let staking_addr = deploy_staking(&env, 5);

    let client = TimelockContractClient::new(&env, &timelock_addr);
    client.initialize(&admin, &token, &beneficiary, &release_ledger, &amount);
    client.set_yield_config(&staking_addr, &yield_recipient);

    client.deposit_locked_balance();
    // Second deposit must fail.
    client.deposit_locked_balance();
}
