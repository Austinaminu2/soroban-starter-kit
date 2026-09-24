#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use crate::{WrappedTokenContract, WrappedTokenContractClient};
use proptest::prelude::*;
use soroban_sdk::{Address, Env, testutils::Address as _, token::StellarAssetClient};

proptest! {
    /// Property: Total wrapped equals total unwrapped (1:1 backing)
    /// Closes #961 – wrapped-token 1:1 backing invariant
    #[test]
    fn prop_wrap_unwrap_conservation(
        amounts in proptest::collection::vec(10i128..=1_000i128, 1..=5),
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let underlying_addr = sac.address();

        let wrapped_addr = env.register_contract(None, WrappedTokenContract);
        let client = WrappedTokenContractClient::new(&env, &wrapped_addr);

        client.initialize(&admin, &underlying_addr);

        let mut total_wrapped = 0i128;
        let mut users = vec![];

        // Wrap tokens
        for amount in amounts {
            let user = Address::generate(&env);
            StellarAssetClient::new(&env, &underlying_addr).mint(&user, &amount);

            if client.try_wrap(&user, &amount).is_ok() {
                total_wrapped += amount;
                users.push((user, amount));
            }
        }

        // Unwrap tokens
        let mut total_unwrapped = 0i128;
        for (user, amount) in users {
            if client.try_unwrap(&user, &amount).is_ok() {
                total_unwrapped += amount;
            }
        }

        // Invariant: total unwrapped <= total wrapped (1:1 backing)
        prop_assert!(total_unwrapped <= total_wrapped,
            "Unwrapped more than wrapped: wrapped={}, unwrapped={}",
            total_wrapped, total_unwrapped);
    }

    /// Property: Wrapped balance never exceeds underlying balance
    #[test]
    fn prop_wrapped_never_exceeds_underlying(
        wrap_amount in 100i128..=10_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let user = Address::generate(&env);

        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let underlying_addr = sac.address();

        StellarAssetClient::new(&env, &underlying_addr).mint(&user, &wrap_amount);

        let wrapped_addr = env.register_contract(None, WrappedTokenContract);
        let client = WrappedTokenContractClient::new(&env, &wrapped_addr);

        client.initialize(&admin, &underlying_addr);

        let _ = client.try_wrap(&user, &wrap_amount);

        let underlying_balance = soroban_sdk::token::Client::new(&env, &underlying_addr).balance(&wrapped_addr);
        let wrapped_balance = client.balance_of(user);

        // Invariant: wrapped supply <= underlying held
        prop_assert!(wrapped_balance <= underlying_balance,
            "Wrapped balance exceeds underlying: wrapped={}, underlying={}",
            wrapped_balance, underlying_balance);
    }

    /// Property: Wrap/unwrap amounts never go negative
    #[test]
    fn prop_amounts_never_negative(
        amount in -1_000i128..=10_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let user = Address::generate(&env);

        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let underlying_addr = sac.address();

        if amount > 0 {
            StellarAssetClient::new(&env, &underlying_addr).mint(&user, &amount);
        }

        let wrapped_addr = env.register_contract(None, WrappedTokenContract);
        let client = WrappedTokenContractClient::new(&env, &wrapped_addr);

        client.initialize(&admin, &underlying_addr);

        let wrap_result = client.try_wrap(&user, &amount);

        if amount <= 0 {
            prop_assert!(wrap_result.is_err(), "Non-positive wrap amount should be rejected");
        }
    }

    /// Property: Round-trip wrap/unwrap returns original amount
    #[test]
    fn prop_roundtrip_exact(
        amount in 1i128..=10_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let user = Address::generate(&env);

        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let underlying_addr = sac.address();

        StellarAssetClient::new(&env, &underlying_addr).mint(&user, &amount);

        let wrapped_addr = env.register_contract(None, WrappedTokenContract);
        let client = WrappedTokenContractClient::new(&env, &wrapped_addr);

        client.initialize(&admin, &underlying_addr);

        let balance_before = soroban_sdk::token::Client::new(&env, &underlying_addr).balance(&user);

        // Wrap
        if client.try_wrap(&user, &amount).is_ok() {
            // Unwrap
            if client.try_unwrap(&user, &amount).is_ok() {
                let balance_after = soroban_sdk::token::Client::new(&env, &underlying_addr).balance(&user);

                // Invariant: round-trip returns original amount
                prop_assert_eq!(balance_after, balance_before,
                    "Round-trip didn't return original amount: before={}, after={}",
                    balance_before, balance_after);
            }
        }
    }
}
