#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use std::format;

use proptest::prelude::*;
use soroban_sdk::{
    Address, Env,
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
};

use crate::{SwapContract, SwapContractClient, SwapState};

fn register_token(env: &Env) -> Address {
    let admin = Address::generate(env);
    env.register_stellar_asset_contract_v2(admin).address()
}

fn mint(env: &Env, token: &Address, to: &Address, amount: i128) {
    StellarAssetClient::new(env, token).mint(to, &amount);
}

proptest! {
    #[test]
    fn prop_sequential_partial_fills_reach_exact_total(
        amount_a in 100i128..=10_000i128,
        ratio in 1i128..=20i128,
        first_fill in 1i128..=5_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let token_a = register_token(&env);
        let token_b = register_token(&env);
        let party_a = Address::generate(&env);
        let party_b = Address::generate(&env);
        let treasury = Address::generate(&env);

        let amount_b = amount_a * ratio;
        mint(&env, &token_a, &party_a, amount_a * 2);
        mint(&env, &token_b, &party_b, amount_b * 2);

        let contract_addr = env.register_contract(None, SwapContract);
        let client = SwapContractClient::new(&env, &contract_addr);
        client.initialize(&party_a, &treasury, &0);
        let approve_until = env.ledger().sequence() + 1_000_000;
        TokenClient::new(&env, &token_a).approve(&party_a, &contract_addr, &(amount_a * 4), &approve_until);

        let expires_at = env.ledger().sequence() + 100;
        let swap_id = client.propose_swap_with_options(
            &party_a,
            &token_a,
            &amount_a,
            &token_b,
            &amount_b,
            &expires_at,
            &true,
            &false,
        );

        let first = first_fill.min(amount_a - 1);
        client.accept_swap_partial(&party_b, &swap_id, &first);
        let second = amount_a - first;
        client.accept_swap_partial(&party_b, &swap_id, &second);

        let swap = client.get_swap(&swap_id);
        prop_assert_eq!(swap.filled_amount, amount_a);
        prop_assert_eq!(swap.state, SwapState::Executed);
        prop_assert_eq!(TokenClient::new(&env, &token_a).balance(&party_b), amount_a);
    }

    #[test]
    fn prop_escrow_cancel_returns_remaining_balance(
        amount_a in 500i128..=20_000i128,
        amount_b in 500i128..=20_000i128,
        fill in 1i128..=10_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let token_a = register_token(&env);
        let token_b = register_token(&env);
        let party_a = Address::generate(&env);
        let party_b = Address::generate(&env);
        let treasury = Address::generate(&env);

        mint(&env, &token_a, &party_a, amount_a * 3);
        mint(&env, &token_b, &party_b, amount_b * 3);

        let contract_addr = env.register_contract(None, SwapContract);
        let client = SwapContractClient::new(&env, &contract_addr);
        client.initialize(&party_a, &treasury, &0);

        let token_a_client = TokenClient::new(&env, &token_a);
        let before = token_a_client.balance(&party_a);
        let expires_at = env.ledger().sequence() + 100;
        let swap_id = client.propose_swap_with_options(
            &party_a,
            &token_a,
            &amount_a,
            &token_b,
            &amount_b,
            &expires_at,
            &true,
            &true,
        );

        client.cancel_swap(&swap_id);

        let after = token_a_client.balance(&party_a);
        let _ = (party_b, fill);
        prop_assert_eq!(after, before);
    }
}
