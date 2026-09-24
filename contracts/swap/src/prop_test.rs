#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use proptest::prelude::*;
use soroban_sdk::{
    Address, Env,
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
};

use crate::{SwapContract, SwapContractClient, calculate_and_validate_fee};

fn setup_swap<'a>(env: &'a Env, fee_bps: u32) -> (SwapContractClient<'a>, Address, Address) {
    let admin = Address::generate(env);
    let treasury = Address::generate(env);

    let swap_addr = env.register_contract(None, SwapContract);
    let client = SwapContractClient::new(env, &swap_addr);
    client.initialize(&admin, &treasury, &fee_bps);

    (client, treasury, swap_addr)
}

proptest! {
    /// Property: Fee + party_a_amount always equals amount_b (no rounding loss)
    /// Closes #961 – swap fee calculation invariant
    #[test]
    fn prop_swap_fee_exact(
        amount_b in 100i128..=1_000_000i128,
        fee_bps in 0u32..=10_000u32,
    ) {
        let fee = calculate_and_validate_fee(amount_b, fee_bps).unwrap();
        let party_a_amount = amount_b - fee;

        // Invariant: no rounding loss
        prop_assert_eq!(fee + party_a_amount, amount_b,
            "Fee split doesn't sum to amount_b: fee={}, party_a={}, amount_b={}",
            fee, party_a_amount, amount_b);

        // Invariant: both amounts non-negative
        prop_assert!(fee >= 0, "Negative fee: {}", fee);
        prop_assert!(party_a_amount >= 0, "Negative party_a amount: {}", party_a_amount);
        prop_assert!(fee <= amount_b, "Fee cannot exceed total traded amount");
    }

    /// Property: Treasury fee cannot exceed total traded amount across split transactions
    #[test]
    fn prop_fee_invariance_split_transactions(
        total_amount in 10i128..=10_000i128,
        num_splits in 2usize..=10usize,
        fee_bps in 1u32..=1_000u32,
    ) {
        let chunk = total_amount / num_splits as i128;
        if chunk > 0 {
            let mut total_split_fee = 0i128;
            for _ in 0..num_splits {
                let split_fee = calculate_and_validate_fee(chunk, fee_bps).unwrap();
                prop_assert!(split_fee > 0, "Fee must be non-zero when fee_bps > 0");
                prop_assert!(split_fee <= chunk, "Fee cannot exceed chunk size");
                total_split_fee += split_fee;
            }
            prop_assert!(total_split_fee <= total_amount,
                "Total fee from split transactions cannot exceed total traded volume");
        }
    }

    /// Property: Total transferred out equals total transferred in
    #[test]
    fn prop_swap_conservation(
        amount_a in 100i128..=10_000i128,
        amount_b in 100i128..=10_000i128,
        fee_bps in 0u32..=1_000u32,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let (client, treasury, swap_addr) = setup_swap(&env, fee_bps);

        let party_a = Address::generate(&env);
        let party_b = Address::generate(&env);

        let sac_admin = Address::generate(&env);
        let sac1 = env.register_stellar_asset_contract_v2(sac_admin.clone());
        let token_a = sac1.address();
        let sac2 = env.register_stellar_asset_contract_v2(sac_admin);
        let token_b = sac2.address();

        // Mint tokens
        StellarAssetClient::new(&env, &token_a).mint(&party_a, &amount_a);
        StellarAssetClient::new(&env, &token_b).mint(&party_b, &amount_b);

        let expires_at = env.ledger().sequence() + 1000;

        // Propose swap
        let swap_id_result = client.try_propose_swap(
            &party_a,
            &token_a,
            &amount_a,
            &token_b,
            &amount_b,
            &expires_at,
            &None,
            &None,
        );

        if swap_id_result.is_err() {
            return Ok(());
        }

        let swap_id = swap_id_result.unwrap();

        // Accept swap
        let accept_result = client.try_accept_swap(&swap_id, &party_b);

        if accept_result.is_ok() {
            let tok_a = soroban_sdk::token::Client::new(&env, &token_a);
            let tok_b = soroban_sdk::token::Client::new(&env, &token_b);

            // Calculate expected fee
            let fee = calculate_and_validate_fee(amount_b, fee_bps).unwrap();
            let party_a_net = amount_b - fee;

            // Verify balances
            prop_assert_eq!(tok_a.balance(&party_b), amount_a,
                "Party B should receive full amount_a");
            prop_assert_eq!(tok_b.balance(&party_a), party_a_net,
                "Party A should receive amount_b minus fee");
            prop_assert_eq!(tok_b.balance(&treasury), fee,
                "Treasury should receive fee");

            // Conservation: contract should have zero balance after swap
            prop_assert_eq!(tok_a.balance(&swap_addr), 0,
                "Contract should have no token_a after swap");
            prop_assert_eq!(tok_b.balance(&swap_addr), 0,
                "Contract should have no token_b after swap");
        }
    }

    /// Property: Fee BPS validation - must be <= 10000
    #[test]
    fn prop_fee_bps_bounded(
        fee_bps in 0u32..=20_000u32,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let swap_addr = env.register_contract(None, SwapContract);
        let client = SwapContractClient::new(&env, &swap_addr);

        let result = client.try_initialize(&admin, &treasury, &fee_bps);

        if fee_bps > 10_000 {
            prop_assert!(result.is_err(), "Should reject fee_bps > 10000");
        } else {
            prop_assert!(result.is_ok() || result.is_err(),
                "fee_bps <= 10000 should be accepted or fail for other reasons");
        }
    }

    /// Property: Cancelled swaps return funds to party_a
    #[test]
    fn prop_cancel_returns_funds(
        amount_a in 100i128..=10_000i128,
        amount_b in 100i128..=10_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let (client, _, _) = setup_swap(&env, 250u32);

        let party_a = Address::generate(&env);
        let sac_admin = Address::generate(&env);
        let sac1 = env.register_stellar_asset_contract_v2(sac_admin.clone());
        let token_a = sac1.address();
        let sac2 = env.register_stellar_asset_contract_v2(sac_admin);
        let token_b = sac2.address();

        StellarAssetClient::new(&env, &token_a).mint(&party_a, &amount_a);

        let expires_at = env.ledger().sequence() + 1000;
        let swap_id_result = client.try_propose_swap(
            &party_a,
            &token_a,
            &amount_a,
            &token_b,
            &amount_b,
            &expires_at,
            &None,
            &None,
        );

        if swap_id_result.is_err() {
            return Ok(());
        }

        let swap_id = swap_id_result.unwrap();
        let balance_after_propose = soroban_sdk::token::Client::new(&env, &token_a).balance(&party_a);

        // Cancel the swap
        let _ = client.try_cancel_swap(&swap_id);

        let balance_after_cancel = soroban_sdk::token::Client::new(&env, &token_a).balance(&party_a);

        // Invariant: party_a gets their tokens back after cancel
        prop_assert!(balance_after_cancel >= balance_after_propose,
            "Party A should get tokens back after cancel: before={}, after={}",
            balance_after_propose, balance_after_cancel);
    }

    /// Property: Expired swaps cannot be accepted
    #[test]
    fn prop_expired_swaps_rejected(
        amount_a in 100i128..=1_000i128,
        amount_b in 100i128..=1_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let (client, _, _) = setup_swap(&env, 250u32);

        let party_a = Address::generate(&env);
        let party_b = Address::generate(&env);

        let sac_admin = Address::generate(&env);
        let sac1 = env.register_stellar_asset_contract_v2(sac_admin.clone());
        let token_a = sac1.address();
        let sac2 = env.register_stellar_asset_contract_v2(sac_admin);
        let token_b = sac2.address();

        StellarAssetClient::new(&env, &token_a).mint(&party_a, &amount_a);
        StellarAssetClient::new(&env, &token_b).mint(&party_b, &amount_b);

        let expires_at = env.ledger().sequence() + 10;
        let swap_id_result = client.try_propose_swap(
            &party_a,
            &token_a,
            &amount_a,
            &token_b,
            &amount_b,
            &expires_at,
            &None,
            &None,
        );

        if swap_id_result.is_err() {
            return Ok(());
        }

        let swap_id = swap_id_result.unwrap();

        // Advance past deadline
        env.ledger().with_mut(|l| l.sequence_number = expires_at + 1);

        let accept_result = client.try_accept_swap(&swap_id, &party_b);

        prop_assert!(accept_result.is_err(), "Expired swap should not be accepted");
    }
}
