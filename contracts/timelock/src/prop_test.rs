#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use crate::{TimelockContract, TimelockContractClient};
use proptest::prelude::*;
use soroban_sdk::{Address, Env, testutils::Address as _, token::StellarAssetClient};

proptest! {
    /// Property: Funds cannot be withdrawn before unlock time
    /// Closes #961 – timelock temporal invariant
    #[test]
    fn prop_no_early_withdrawal(
        amount in 100i128..=10_000i128,
        lock_duration in 10u32..=1_000u32,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let depositor = Address::generate(&env);
        let beneficiary = Address::generate(&env);

        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let token_addr = sac.address();

        StellarAssetClient::new(&env, &token_addr).mint(&depositor, &amount);

        let timelock_addr = env.register_contract(None, TimelockContract);
        let client = TimelockContractClient::new(&env, &timelock_addr);

        let unlock_time = env.ledger().sequence() + lock_duration;
        client.initialize(&depositor, &beneficiary, &token_addr, &amount, &unlock_time);

        // Try to withdraw before unlock time
        let early_ledger = unlock_time - 1;
        env.ledger().with_mut(|l| l.sequence_number = early_ledger);

        let early_withdraw = client.try_withdraw();

        // Should fail before unlock time
        prop_assert!(early_withdraw.is_err(),
            "Withdrawal should fail before unlock time");
    }

    /// Property: Deposited amount never goes negative
    #[test]
    fn prop_amount_never_negative(
        amount in -1_000i128..=10_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let depositor = Address::generate(&env);
        let beneficiary = Address::generate(&env);

        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let token_addr = sac.address();

        if amount > 0 {
            StellarAssetClient::new(&env, &token_addr).mint(&depositor, &amount);
        }

        let timelock_addr = env.register_contract(None, TimelockContract);
        let client = TimelockContractClient::new(&env, &timelock_addr);

        let unlock_time = env.ledger().sequence() + 100;
        let result = client.try_initialize(&depositor, &beneficiary, &token_addr, &amount, &unlock_time);

        if amount <= 0 {
            prop_assert!(result.is_err(), "Non-positive amount should be rejected");
        }
    }

    /// Property: Unlock time must be in the future
    #[test]
    fn prop_unlock_time_future(
        time_offset in -100i32..=100i32,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 1000);

        let depositor = Address::generate(&env);
        let beneficiary = Address::generate(&env);

        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let token_addr = sac.address();

        StellarAssetClient::new(&env, &token_addr).mint(&depositor, &1000i128);

        let timelock_addr = env.register_contract(None, TimelockContract);
        let client = TimelockContractClient::new(&env, &timelock_addr);

        let current = env.ledger().sequence();
        let unlock_time = if time_offset >= 0 {
            current + time_offset as u32
        } else {
            current.saturating_sub((-time_offset) as u32)
        };

        let result = client.try_initialize(&depositor, &beneficiary, &token_addr, &1000i128, &unlock_time);

        if unlock_time <= current {
            prop_assert!(result.is_err(), "Past unlock time should be rejected");
        }
    }
}
