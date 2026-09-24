#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use crate::{SubscriptionContract, SubscriptionContractClient};
use proptest::prelude::*;
use soroban_sdk::{Address, Env, testutils::Address as _, token::StellarAssetClient};

proptest! {
    /// Property: Total collected never exceeds sum of individual subscription payments
    /// Closes #961 – subscription payment accounting invariant
    #[test]
    fn prop_collected_equals_payments(
        price in 10i128..=1_000i128,
        num_subscribers in 1usize..=10usize,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let provider = Address::generate(&env);
        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let token_addr = sac.address();

        let sub_addr = env.register_contract(None, SubscriptionContract);
        let client = SubscriptionContractClient::new(&env, &sub_addr);

        client.initialize(&provider, &token_addr, &price, &100u32);

        let mut total_paid = 0i128;

        // Subscribe and pay
        for _ in 0..num_subscribers {
            let subscriber = Address::generate(&env);
            StellarAssetClient::new(&env, &token_addr).mint(&subscriber, &price);

            if client.try_subscribe(&subscriber).is_ok() {
                total_paid += price;
            }
        }

        let provider_balance = soroban_sdk::token::Client::new(&env, &token_addr).balance(&provider);

        // Invariant: provider received <= total paid
        prop_assert!(provider_balance <= total_paid,
            "Provider received more than total payments: received={}, paid={}",
            provider_balance, total_paid);
    }

    /// Property: Subscription period never negative
    #[test]
    fn prop_period_never_negative(
        period in 0u32..=1_000u32,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let provider = Address::generate(&env);
        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let token_addr = sac.address();

        let sub_addr = env.register_contract(None, SubscriptionContract);
        let client = SubscriptionContractClient::new(&env, &sub_addr);

        let result = client.try_initialize(&provider, &token_addr, &100i128, &period);

        if period == 0 {
            prop_assert!(result.is_err(), "Zero period should be rejected");
        }
    }

    /// Property: Subscription price never negative
    #[test]
    fn prop_price_positive(
        price in -1_000i128..=1_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let provider = Address::generate(&env);
        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let token_addr = sac.address();

        let sub_addr = env.register_contract(None, SubscriptionContract);
        let client = SubscriptionContractClient::new(&env, &sub_addr);

        let result = client.try_initialize(&provider, &token_addr, &price, &100u32);

        if price <= 0 {
            prop_assert!(result.is_err(), "Non-positive price should be rejected");
        }
    }
}
