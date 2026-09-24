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
use soroban_sdk::{Address, Env, String, testutils::Address as _};

use crate::TokenContract;
use crate::TokenContractClient;

fn setup(env: &Env) -> (TokenContractClient<'_>, Address) {
    let admin = Address::generate(env);
    let addr = env.register_contract(None, TokenContract);
    let client = TokenContractClient::new(env, &addr);
    client.initialize(
        &admin,
        &String::from_str(env, "Test Token"),
        &String::from_str(env, "TEST"),
        &18u32,
        &None,
    );
    (client, admin)
}

proptest! {
    /// Mint then burn the same amount → balance returns to zero.
    #[test]
    fn prop_mint_burn_roundtrip(amount in 1i128..=i128::MAX / 2) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _) = setup(&env);
        let user = Address::generate(&env);

        client.mint(&user, &amount);
        prop_assert_eq!(client.balance(&user), amount);
        prop_assert_eq!(client.total_supply(), amount);

        client.admin_burn(&user, &amount);
        prop_assert_eq!(client.balance(&user), 0);
        prop_assert_eq!(client.total_supply(), 0);
    }

    /// Transfer is conservative: sender loses exactly what receiver gains.
    /// Total supply is unchanged and all balances stay non-negative.
    #[test]
    fn prop_transfer_conservation(
        mint in 1i128..=1_000_000i128,
        transfer in 1i128..=1_000_000i128,
    ) {
        let transfer = transfer.min(mint);
        let env = Env::default();
        env.mock_all_auths();
        let (client, _) = setup(&env);
        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);

        client.mint(&sender, &mint);
        client.transfer(&sender, &receiver, &transfer);

        prop_assert_eq!(client.balance(&sender), mint - transfer);
        prop_assert_eq!(client.balance(&receiver), transfer);
        prop_assert_eq!(client.total_supply(), mint);
        prop_assert!(client.balance(&sender) >= 0);
        prop_assert!(client.balance(&receiver) >= 0);
    }

    /// User burn reduces balance and total supply; balances never go negative.
    #[test]
    fn prop_burn_conserves_supply(
        mint in 1i128..=1_000_000i128,
        burn in 1i128..=1_000_000i128,
    ) {
        let burn = burn.min(mint);
        let env = Env::default();
        env.mock_all_auths();
        let (client, _) = setup(&env);
        let user = Address::generate(&env);

        client.mint(&user, &mint);
        client.burn(&user, &burn);

        prop_assert_eq!(client.balance(&user), mint - burn);
        prop_assert_eq!(client.total_supply(), mint - burn);
        prop_assert!(client.balance(&user) >= 0);
    }

    /// transfer_from decrements allowance and preserves total supply.
    #[test]
    fn prop_transfer_from_conserves_supply(
        mint in 1i128..=1_000_000i128,
        approve in 1i128..=1_000_000i128,
        spend in 1i128..=1_000_000i128,
    ) {
        let approve = approve.min(mint);
        let spend = spend.min(approve);
        let env = Env::default();
        env.mock_all_auths();
        let (client, _) = setup(&env);
        let owner = Address::generate(&env);
        let spender = Address::generate(&env);
        let receiver = Address::generate(&env);

        client.mint(&owner, &mint);
        let expiry = env.ledger().sequence() + 1000;
        client.approve(&owner, &spender, &approve, &expiry);
        client.transfer_from(&spender, &owner, &receiver, &spend);

        prop_assert_eq!(client.allowance(&owner, &spender), approve - spend);
        prop_assert_eq!(client.balance(&owner), mint - spend);
        prop_assert_eq!(client.balance(&receiver), spend);
        prop_assert_eq!(client.total_supply(), mint);
        prop_assert!(client.balance(&owner) >= 0);
        prop_assert!(client.balance(&receiver) >= 0);
    }

    /// Total supply equals the sum of all individual balances after arbitrary mints.
    #[test]
    fn prop_total_supply_matches_sum_of_balances(
        amounts in proptest::collection::vec(1i128..=100_000i128, 1..=10),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _) = setup(&env);

        let mut total = 0i128;
        for amount in &amounts {
            let user = Address::generate(&env);
            client.mint(&user, amount);
            total += amount;
        }
        prop_assert_eq!(client.total_supply(), total);
    }

    /// Sequential mints never exceed max_supply cap.
    #[test]
    #[cfg(feature = "capped-supply")]
    fn prop_total_supply_never_exceeds_cap(
        amounts in proptest::collection::vec(1i128..=100_000i128, 1..=20),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let addr = env.register_contract(None, TokenContract);
        let client = TokenContractClient::new(&env, &addr);
        let max_supply = 1_000_000i128;
        client.initialize(
            &admin,
            &String::from_str(&env, "Capped Token"),
            &String::from_str(&env, "CAP"),
            &18u32,
            &Some(max_supply),
        );

        let mut total = 0i128;
        for amount in &amounts {
            let user = Address::generate(&env);
            let mint_amount = (*amount).min(max_supply.saturating_sub(total));
            if mint_amount > 0 {
                client.mint(&user, &mint_amount);
                total += mint_amount;
            }
            prop_assert!(client.total_supply() <= max_supply);
        }
    }
}
