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

use crate::{OracleContract, OracleContractClient};

fn setup_oracle<'a>(env: &'a Env) -> (OracleContractClient<'a>, Address) {
    let admin = Address::generate(env);
    let oracle_addr = env.register(OracleContract, ());
    let client = OracleContractClient::new(env, &oracle_addr);
    client.initialize(&admin, &100u32);
    (client, admin)
}

proptest! {
    /// Any positive price should be accepted and retrievable without stale price error.
    #[test]
    fn prop_positive_price_accepted(price in 1i128..=i128::MAX) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_oracle(&env);

        let result = client.try_update_price(&price);
        prop_assert!(result.is_ok());

        let retrieved = client.get_price();
        prop_assert_eq!(retrieved, price);
    }

    /// Zero and negative prices must be rejected with InvalidPrice (#1258).
    #[test]
    fn prop_non_positive_price_rejected(price in i128::MIN..=0i128) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_oracle(&env);

        let result = client.try_update_price(&price);
        prop_assert!(result.is_err());
    }

    /// Rapid sequential positive price updates should always leave get_price consistent
    /// with the last accepted value.
    #[test]
    fn prop_rapid_sequential_updates_consistent(
        prices in proptest::collection::vec(1i128..=i128::MAX, 1..=20),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_oracle(&env);

        let mut last_price = 0i128;

        for price in prices.iter() {
            env.ledger().with_mut(|l| {
                l.timestamp += 1;
                l.sequence_number += 1;
            });

            let result = client.try_update_price(price);
            prop_assert!(result.is_ok());
            last_price = *price;
        }

        let final_price = client.get_price();
        prop_assert_eq!(final_price, last_price);
    }

    /// get_price and get_price_data should always return the same price value.
    #[test]
    fn prop_get_price_consistency(price in 1i128..=1_000_000i128) {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_oracle(&env);

        client.update_price(&price);

        let price_from_get = client.get_price();
        let price_from_data = client.get_price_data().price;

        prop_assert_eq!(price_from_get, price_from_data);
        prop_assert_eq!(price_from_get, price);
    }
}
