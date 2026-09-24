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
};
use std::format;

use crate::{OracleContract, OracleContractClient};

fn setup_oracle<'a>(env: &'a Env) -> (OracleContractClient<'a>, Address) {
    let admin = Address::generate(env);

    let oracle_addr = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(env, &oracle_addr);

    let staleness_threshold = 100u32;
    client.initialize(&admin, &staleness_threshold);

    (client, admin)
}

/// Zero price updates should be accepted and retrievable
#[test]
fn prop_zero_price_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup_oracle(&env);

    let result = client.try_update_price(&0i128);
    assert!(result.is_ok());

    let price = client.get_price();
    assert_eq!(price, 0i128);
}

/// Maximum i128 price should be accepted and retrievable
#[test]
fn prop_max_price_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup_oracle(&env);

    let max_price = i128::MAX;
    let result = client.try_update_price(&max_price);
    assert!(result.is_ok());

    let price = client.get_price();
    assert_eq!(price, max_price);
}

/// Minimum i128 price should be accepted and retrievable
#[test]
fn prop_min_price_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin) = setup_oracle(&env);

    let min_price = i128::MIN;
    let result = client.try_update_price(&min_price);
    assert!(result.is_ok());

    let price = client.get_price();
    assert_eq!(price, min_price);
}

proptest! {
    /// Price updates at boundary values (zero, max i128) should not panic
    /// and get_price/get_price_data should remain consistent.
    /// Closes #859 – proptest for oracle price update boundary conditions
    #[test]
    fn prop_boundary_price_updates_no_panic(price in prop::option::of(prop::num::i128::ANY)) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_oracle(&env);

        // Test with boundary or random price
        let test_price = price.unwrap_or(0i128);

        // Update should not panic
        let update_result = client.try_update_price(&test_price);

        if update_result.is_ok() {
            // If update succeeds, get_price should return the same value
            let retrieved_price = client.try_get_price();
            prop_assert!(retrieved_price.is_ok());
            prop_assert_eq!(retrieved_price.unwrap(), test_price);

            prop_assert_eq!(retrieved_price.unwrap().unwrap(), test_price);
            
            // get_price_data should also be consistent
            let price_data = client.try_get_price_data();
            prop_assert!(price_data.is_ok());
            prop_assert_eq!(price_data.unwrap().unwrap().price, test_price);
        }
    }

    /// Zero price updates should be accepted and retrievable
    #[test]
    fn prop_zero_price_accepted() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_oracle(&env);

        let result = client.try_update_price(&0i128);
        prop_assert!(result.is_ok());

        let price = client.get_price().unwrap();
        prop_assert_eq!(price, 0i128);
    }

    /// Maximum i128 price should be accepted and retrievable
    #[test]
    fn prop_max_price_accepted() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_oracle(&env);

        let max_price = i128::MAX;
        let result = client.try_update_price(&max_price);
        prop_assert!(result.is_ok());

        let price = client.get_price().unwrap();
        prop_assert_eq!(price, max_price);
    }

    /// Minimum i128 price should be accepted and retrievable
    #[test]
    fn prop_min_price_accepted() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_oracle(&env);

        let min_price = i128::MIN;
        let result = client.try_update_price(&min_price);
        prop_assert!(result.is_ok());

        let price = client.get_price().unwrap();
        prop_assert_eq!(price, min_price);
    }

    /// Rapid sequential updates should maintain consistency
    #[test]
    fn prop_rapid_sequential_updates_consistent(
        prices in proptest::collection::vec(prop::num::i128::ANY, 1..=20),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_oracle(&env);

        let mut last_successful_price = None;

        for (idx, price) in prices.iter().enumerate() {
            // Advance timestamp slightly for each update
            env.ledger().with_mut(|l| {
                l.timestamp += 1;
                l.sequence_number += 1;
            });

            let result = client.try_update_price(price);

            if result.is_ok() {
                last_successful_price = Some(*price);

                // Verify get_price matches what we just set
                let retrieved = client.try_get_price();
                prop_assert!(retrieved.is_ok(), "get_price failed after update {}", idx);
                prop_assert_eq!(retrieved.unwrap(), *price, "Price mismatch at update {}", idx);

                prop_assert_eq!(retrieved.unwrap().unwrap(), *price, "Price mismatch at update {}", idx);
                
                // Verify get_price_data is consistent
                let price_data = client.try_get_price_data();
                prop_assert!(price_data.is_ok(), "get_price_data failed after update {}", idx);
                prop_assert_eq!(price_data.unwrap().unwrap().price, *price, "Price data mismatch at update {}", idx);
            }
        }

        // Final check: last successful update should still be retrievable
        if let Some(expected_price) = last_successful_price {
            let final_price = client.get_price();
            prop_assert_eq!(final_price, expected_price);
        }
    }

    /// get_price and get_price_data should always return consistent values
    #[test]
    fn prop_get_price_consistency(price in -1_000_000i128..=1_000_000i128) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_oracle(&env);

        client.update_price(&price);

        let price_from_get = client.get_price().unwrap();
        let price_from_data = client.get_price_data().unwrap().price;

        
        let price_from_get = client.get_price();
        let price_from_data = client.get_price_data().price;
        
        prop_assert_eq!(price_from_get, price_from_data);
        prop_assert_eq!(price_from_get, price);
    }
}
