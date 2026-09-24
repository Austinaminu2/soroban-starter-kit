#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use std::format;
use std::vec;

use proptest::prelude::*;
use soroban_sdk::{
    Address, Env,
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
};

use crate::{AuctionContract, AuctionContractClient};

fn setup_auction<'a>(
    env: &'a Env,
    start_price: i128,
    min_increment: i128,
) -> (AuctionContractClient<'a>, Address, Address, Address) {
    let seller = Address::generate(env);
    let sac_admin = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(sac_admin);
    let token_addr = sac.address();

    let auction_addr = env.register_contract(None, AuctionContract);
    let client = AuctionContractClient::new(env, &auction_addr);

    let deadline = env.ledger().sequence() + 1000;
    client.start(
        &seller,
        &token_addr,
        &start_price,
        &min_increment,
        &deadline,
        &None,
        &0,
    );

    (client, seller, token_addr, auction_addr)
}

proptest! {
    /// Property: Total funds withdrawn by bidders can never exceed total funds deposited via bids
    /// Closes #961 – auction bid/refund accounting invariant
    #[test]
    fn prop_auction_total_out_never_exceeds_total_in(
        bids in proptest::collection::vec(100i128..=10_000i128, 1..=10),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let (client, seller, token_addr, auction_addr) = setup_auction(&env, 50i128, 10i128);

        let mut bidders = vec![];
        let mut total_deposited = 0i128;

        // Place bids
        for bid_amount in bids {
            let bidder = Address::generate(&env);
            StellarAssetClient::new(&env, &token_addr).mint(&bidder, &bid_amount);

            let result = client.try_bid(&bidder, &bid_amount);
            if result.is_ok() {
                total_deposited += bid_amount;
                bidders.push(bidder);
            }
        }

        // Withdraw all pending refunds
        let mut total_withdrawn = 0i128;
        for bidder in &bidders {
            let pending = client.get_pending(bidder);
            if pending > 0 {
                let balance_before = soroban_sdk::token::Client::new(&env, &token_addr).balance(bidder);
                let withdraw_result = client.try_withdraw(bidder);
                if withdraw_result.is_ok() {
                    let balance_after = soroban_sdk::token::Client::new(&env, &token_addr).balance(bidder);
                    total_withdrawn += balance_after - balance_before;
                }
            }
        }

        // End auction and withdraw winner
        env.ledger().with_mut(|l| l.sequence_number += 1001);
        let _ = client.try_end();

        // Check final balances
        for bidder in &bidders {
            let pending = client.get_pending(bidder);
            if pending > 0 {
                let balance_before = soroban_sdk::token::Client::new(&env, &token_addr).balance(bidder);
                let _ = client.try_withdraw(bidder);
                let balance_after = soroban_sdk::token::Client::new(&env, &token_addr).balance(bidder);
                total_withdrawn += balance_after - balance_before;
            }
        }

        // Invariant: total withdrawn <= total deposited
        prop_assert!(total_withdrawn <= total_deposited,
            "Refunds exceeded deposits: deposited={}, withdrawn={}",
            total_deposited, total_withdrawn);

        // Invariant: contract balance + total_withdrawn + seller_payout = total_deposited
        let contract_balance = soroban_sdk::token::Client::new(&env, &token_addr).balance(&auction_addr);
        let seller_payout = soroban_sdk::token::Client::new(&env, &token_addr).balance(&seller);
        prop_assert_eq!(contract_balance + total_withdrawn + seller_payout, total_deposited,
            "Balance accounting mismatch");
    }

    /// Property: Pending refunds never go negative
    #[test]
    fn prop_pending_refunds_never_negative(
        bid_amounts in proptest::collection::vec(50i128..=1_000i128, 2..=5),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let (client, _seller, token_addr, _) = setup_auction(&env, 10i128, 5i128);

        let mut bidders = vec![];
        for bid_amount in bid_amounts {
            let bidder = Address::generate(&env);
            StellarAssetClient::new(&env, &token_addr).mint(&bidder, &bid_amount);

            if client.try_bid(&bidder, &bid_amount).is_ok() {
                bidders.push(bidder);
            }
        }

        // Check all pending amounts are non-negative
        for bidder in &bidders {
            let pending = client.get_pending(bidder);
            prop_assert!(pending >= 0, "Negative pending refund detected: {}", pending);
        }
    }

    /// Property: Highest bid always increases or stays the same
    #[test]
    fn prop_highest_bid_monotonic(
        increments in proptest::collection::vec(10i128..=100i128, 1..=10),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let start_price = 100i128;
        let (client, _seller, token_addr, _) = setup_auction(&env, start_price, 10i128);

        let mut prev_highest = start_price - 1;
        let mut current_bid = start_price;

        for increment in increments {
            let bidder = Address::generate(&env);
            StellarAssetClient::new(&env, &token_addr).mint(&bidder, &current_bid);

            if client.try_bid(&bidder, &current_bid).is_ok() {
                let info = client.get_info();
                prop_assert!(info.highest_bid >= prev_highest,
                    "Highest bid decreased: {} -> {}", prev_highest, info.highest_bid);
                prev_highest = info.highest_bid;
                current_bid = info.highest_bid + increment;
            }
        }
    }

    /// Property: Reserve price prevents seller payout when not met
    #[test]
    fn prop_reserve_price_enforcement(
        start_price in 100i128..=1_000i128,
        reserve_price in 500i128..=2_000i128,
        bid_amount in 100i128..=3_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let seller = Address::generate(&env);
        let bidder = Address::generate(&env);
        let sac_admin = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(sac_admin);
        let token_addr = sac.address();

        let auction_addr = env.register_contract(None, AuctionContract);
        let client = AuctionContractClient::new(&env, &auction_addr);

        let deadline = env.ledger().sequence() + 100;
        let result = client.try_start(
            &seller,
            &token_addr,
            &start_price,
            &10i128,
            &deadline,
            &Some(reserve_price),
            &0
        );

        if result.is_err() {
            return Ok(());
        }

        if bid_amount >= start_price {
            StellarAssetClient::new(&env, &token_addr).mint(&bidder, &bid_amount);
            let _ = client.try_bid(&bidder, &bid_amount);
        }

        // End auction
        env.ledger().with_mut(|l| l.sequence_number = deadline + 1);
        let _ = client.try_end();

        let seller_balance = soroban_sdk::token::Client::new(&env, &token_addr).balance(&seller);

        if bid_amount >= reserve_price && bid_amount >= start_price {
            // Reserve met: seller should receive funds
            prop_assert!(seller_balance > 0 || bid_amount == 0,
                "Seller didn't receive payment when reserve was met");
        } else {
            // Reserve not met: seller should receive nothing
            prop_assert_eq!(seller_balance, 0,
                "Seller received payment when reserve was not met");
        }
    }
}
