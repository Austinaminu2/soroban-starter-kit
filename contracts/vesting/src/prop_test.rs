#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use proptest::prelude::*;
use soroban_sdk::{Env, testutils::Ledger as _};
use std::format;

use super::test::{make_token, setup_env};
use crate::{VestingContract, VestingContractClient, vested_amount};
use soroban_sdk::Address;
use soroban_sdk::testutils::Address as _;

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
    fn prop_vested_amount_matches_schedule_checkpoints(
        amount in 1i128..=1_000_000_000i128,
        cliff_offset in 1u32..=10_000u32,
        duration in 1u32..=100_000u32,
        checkpoint_pct in 0u32..=100u32,
    ) {
        let env = setup_env();
        let start = env.ledger().sequence();
        let cliff = start + cliff_offset;
        let end = cliff + duration;
        let checkpoint = cliff + duration * checkpoint_pct / 100;
        let expected = if checkpoint < cliff {
            0
        } else if checkpoint >= end {
            amount
        } else {
            amount * i128::from(checkpoint - cliff) / i128::from(duration)
        };

        prop_assert_eq!(vested_amount(amount, cliff, end, checkpoint), expected);
    }

    #[test]
    fn prop_vesting_claimable_matches_exact_release_formula(
        amount in 1i128..=1_000_000_000i128,
        cliff_offset in 1u32..=10_000u32,
        duration in 1u32..=100_000u32,
        checkpoint_pct in 0u32..=100u32,
    ) {
        let env = setup_env();
        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = make_token(&env, &admin, amount);
        let cliff = env.ledger().sequence() + cliff_offset;
        let end = cliff + duration;
        let addr = env.register_contract(None, VestingContract);
        let client = VestingContractClient::new(&env, &addr);
        client.initialize(&admin, &token);
        client.create_schedule(&beneficiary, &cliff, &end, &amount);

        let before_cliff = cliff - 1;
        env.ledger().with_mut(|l| l.sequence_number = before_cliff);
        prop_assert_eq!(client.claimable(&beneficiary), 0);

        let partial_checkpoint = cliff + duration * checkpoint_pct / 100;
        env.ledger().with_mut(|l| l.sequence_number = partial_checkpoint);
        prop_assert_eq!(
            client.claimable(&beneficiary),
            vested_amount(amount, cliff, end, partial_checkpoint)
        );

        env.ledger().with_mut(|l| l.sequence_number = end);
        prop_assert_eq!(client.claimable(&beneficiary), amount);
    }

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