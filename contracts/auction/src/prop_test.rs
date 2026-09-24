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

use crate::dutch::price_at;
use crate::{AuctionContract, AuctionContractClient, AuctionError, DataKey, DutchConfig};

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
        &None,
        &None,
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

        let (client, _seller, token_addr, auction_addr) = setup_auction(&env, 50i128, 10i128);

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

        // Invariant: contract balance + total_withdrawn = total_deposited
        let contract_balance = soroban_sdk::token::Client::new(&env, &token_addr).balance(&auction_addr);
        prop_assert_eq!(contract_balance + total_withdrawn, total_deposited,
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
                let info = client.get_info().unwrap();
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
            &0,
            &None,
            &None,
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

proptest! {
    /// Issue #1071: the Dutch price never increases over time, stays within
    /// `[floor_price, start_price]`, and clamps at both ends of the schedule.
    #[test]
    fn prop_dutch_price_monotonic_and_clamped(
        (start_price, floor_price) in (1i128..=i128::MAX).prop_flat_map(|s| (Just(s), 0i128..s)),
        start_ledger in 0u32..=1_000_000,
        duration_ledgers in 1u32..=1_000_000,
        t1 in 0u32..=3_000_000,
        dt in 0u32..=3_000_000,
    ) {
        let cfg = DutchConfig { start_price, floor_price, start_ledger, duration_ledgers };
        let t2 = t1.saturating_add(dt);

        // The split-division formula never overflows for valid schedules.
        let p1 = price_at(&cfg, t1).unwrap();
        let p2 = price_at(&cfg, t2).unwrap();

        prop_assert!(p2 <= p1, "price increased: {} -> {}", p1, p2);
        prop_assert!(p1 >= floor_price && p1 <= start_price);
        if t1 <= start_ledger {
            prop_assert_eq!(p1, start_price);
        }
        if t1 >= start_ledger + duration_ledgers {
            prop_assert_eq!(p1, floor_price);
        }
    }

    /// Issue #1071: for values where the textbook formula cannot overflow, the
    /// contract's price matches it exactly.
    #[test]
    fn prop_dutch_price_matches_reference_formula(
        (start_price, floor_price) in (1i128..=1_000_000_000_000i128)
            .prop_flat_map(|s| (Just(s), 0i128..s)),
        duration_ledgers in 1u32..=100_000,
        elapsed in 0u32..=100_000,
    ) {
        let cfg = DutchConfig { start_price, floor_price, start_ledger: 0, duration_ledgers };
        let expected = if elapsed >= duration_ledgers {
            floor_price
        } else {
            start_price
                - (start_price - floor_price) * i128::from(elapsed) / i128::from(duration_ledgers)
        };
        prop_assert_eq!(price_at(&cfg, elapsed).unwrap(), expected);
    }

    /// Issue #1070: bidding near `i128::MAX` returns `AuctionError::Overflow`
    /// (or a normal validation error) instead of trapping the host.
    #[test]
    fn prop_bid_near_i128_max_returns_error_not_panic(
        headroom in 0i128..=1_000_000i128,
        min_increment in 1i128..=2_000_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let start_price = i128::MAX - headroom;
        let (client, _seller, token_addr, _) = setup_auction(&env, start_price, min_increment);

        let first = Address::generate(&env);
        StellarAssetClient::new(&env, &token_addr).mint(&first, &start_price);
        client.bid(&first, &start_price);

        let second = Address::generate(&env);
        match start_price.checked_add(min_increment) {
            None => {
                prop_assert_eq!(
                    client.try_bid(&second, &i128::MAX),
                    Err(Ok(AuctionError::Overflow))
                );
            }
            Some(required) => {
                prop_assert_eq!(
                    client.try_bid(&second, &(required - 1)),
                    Err(Ok(AuctionError::BidTooLow))
                );
            }
        }
        prop_assert_eq!(client.get_info().highest_bid, start_price);
    }

    /// Issue #1070: queueing a refund onto a pending balance near `i128::MAX`
    /// returns `AuctionError::Overflow` exactly when the sum would overflow.
    #[test]
    fn prop_pending_refund_near_i128_max(existing in (i128::MAX - 2_000)..=i128::MAX) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let (client, _seller, token_addr, auction_addr) = setup_auction(&env, 1_000, 100);
        let b1 = Address::generate(&env);
        let b2 = Address::generate(&env);
        StellarAssetClient::new(&env, &token_addr).mint(&b1, &1_000);
        StellarAssetClient::new(&env, &token_addr).mint(&b2, &1_100);
        client.bid(&b1, &1_000);

        env.as_contract(&auction_addr, || {
            env.storage()
                .persistent()
                .set(&DataKey::Pending(b1.clone()), &existing);
        });

        let result = client.try_bid(&b2, &1_100);
        if existing.checked_add(1_000).is_none() {
            prop_assert_eq!(result, Err(Ok(AuctionError::Overflow)));
            prop_assert_eq!(client.get_pending(&b1), existing);
        } else {
            prop_assert!(result.is_ok());
            prop_assert_eq!(client.get_pending(&b1), existing + 1_000);
        }
    }

    /// Issue #1068: mixing `bid` and `bid_with_credit` keeps the contract exactly
    /// solvent: its balance equals the highest bid plus all pending refunds.
    #[test]
    fn prop_bid_with_credit_preserves_accounting(
        steps in proptest::collection::vec((0i128..=500i128, any::<bool>()), 1..=12),
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.sequence_number = 100);

        let (client, _seller, token_addr, auction_addr) = setup_auction(&env, 100, 10);
        let token = soroban_sdk::token::Client::new(&env, &token_addr);
        let bidders = [Address::generate(&env), Address::generate(&env)];
        for bidder in &bidders {
            StellarAssetClient::new(&env, &token_addr).mint(bidder, &1_000_000);
        }

        let mut next_min = 100i128;
        for (i, (extra, use_credit)) in steps.into_iter().enumerate() {
            let bidder = &bidders[i % 2];
            let amount = next_min + extra;
            if use_credit {
                client.bid_with_credit(bidder, &amount);
            } else {
                client.bid(bidder, &amount);
            }
            next_min = amount + 10;

            let owed = amount + client.get_pending(&bidders[0]) + client.get_pending(&bidders[1]);
            prop_assert_eq!(token.balance(&auction_addr), owed);
            let total = token.balance(&bidders[0]) + token.balance(&bidders[1]) + owed;
            prop_assert_eq!(total, 2_000_000);
        }
    }
}
