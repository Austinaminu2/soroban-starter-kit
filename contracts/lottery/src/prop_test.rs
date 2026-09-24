#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use crate::{LotteryContract, LotteryContractClient};
use proptest::prelude::*;
use soroban_sdk::{Address, Env, testutils::Address as _, token::StellarAssetClient};

proptest! {
    /// Property: Total payouts never exceed total ticket sales
    /// Closes #961 – lottery payout accounting invariant
    #[test]
    fn prop_payouts_never_exceed_sales(
        ticket_price in 10i128..=100i128,
        num_tickets in 1u32..=20u32,
        winner_count in 1u32..=5u32,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let admin = Address::generate(&env);
        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let token_addr = sac.address();

        let lottery_addr = env.register_contract(None, LotteryContract);
        let client = LotteryContractClient::new(&env, &lottery_addr);

        let deadline = env.ledger().sequence() + 100;
        let reveal_deadline = deadline + 50;

        let init_result = client.try_initialize(
            &admin,
            &token_addr,
            &ticket_price,
            &deadline,
            &reveal_deadline,
            &winner_count,
            &None
        );

        if init_result.is_err() {
            return Ok(());
        }

        let mut total_sales = 0i128;
        let mut participants = vec![];

        // Buy tickets
        for _ in 0..num_tickets {
            let participant = Address::generate(&env);
            StellarAssetClient::new(&env, &token_addr).mint(&participant, &ticket_price);

            if client.try_buy_ticket(&participant).is_ok() {
                total_sales += ticket_price;
                participants.push(participant);
            }
        }

        // Advance past deadlines and try to finalize
        env.ledger().with_mut(|l| l.sequence_number = reveal_deadline + 1);

        let contract_balance_before = soroban_sdk::token::Client::new(&env, &token_addr).balance(&lottery_addr);

        // Try to draw (may fail for various reasons)
        let _ = client.try_draw();

        let contract_balance_after = soroban_sdk::token::Client::new(&env, &token_addr).balance(&lottery_addr);
        let total_paid_out = contract_balance_before - contract_balance_after;

        // Invariant: payouts <= sales
        prop_assert!(total_paid_out <= total_sales,
            "Payouts exceeded sales: sales={}, payouts={}",
            total_sales, total_paid_out);
    }

    /// Property: Winner count validation
    #[test]
    fn prop_winner_count_bounded(
        winner_count in 0u32..=100u32,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let token_addr = sac.address();

        let lottery_addr = env.register_contract(None, LotteryContract);
        let client = LotteryContractClient::new(&env, &lottery_addr);

        let deadline = env.ledger().sequence() + 100;
        let reveal_deadline = deadline + 50;

        let result = client.try_initialize(
            &admin,
            &token_addr,
            &100i128,
            &deadline,
            &reveal_deadline,
            &winner_count,
            &None
        );

        // Zero winners should be rejected
        if winner_count == 0 {
            prop_assert!(result.is_err(), "Zero winners should be rejected");
        }
    }

    /// Property: Ticket price never negative
    #[test]
    fn prop_ticket_price_positive(
        ticket_price in -1_000i128..=1_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let token_addr = sac.address();

        let lottery_addr = env.register_contract(None, LotteryContract);
        let client = LotteryContractClient::new(&env, &lottery_addr);

        let deadline = env.ledger().sequence() + 100;
        let reveal_deadline = deadline + 50;

        let result = client.try_initialize(
            &admin,
            &token_addr,
            &ticket_price,
            &deadline,
            &reveal_deadline,
            &1u32,
            &None
        );

        if ticket_price <= 0 {
            prop_assert!(result.is_err(), "Non-positive ticket price should be rejected");
        }
    }
}
