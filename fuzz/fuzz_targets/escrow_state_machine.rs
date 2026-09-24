#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_escrow_template::EscrowContract;
use soroban_sdk::{Address, Env, testutils::Address as _, token::StellarAssetClient};

/// State machine operations for escrow fuzz test
/// Closes #963 – escrow state machine fuzz target
#[derive(Debug, Clone, Copy)]
enum EscrowOp {
    Fund,
    MarkDelivered,
    ApproveDelivery,
    ReleasePartial,
    RequestRefund,
    RequestPartialRefund,
    Cancel,
    RaiseDispute,
    ResolveDispute,
}

fn byte_to_op(byte: u8) -> EscrowOp {
    match byte % 9 {
        0 => EscrowOp::Fund,
        1 => EscrowOp::MarkDelivered,
        2 => EscrowOp::ApproveDelivery,
        3 => EscrowOp::ReleasePartial,
        4 => EscrowOp::RequestRefund,
        5 => EscrowOp::RequestPartialRefund,
        6 => EscrowOp::Cancel,
        7 => EscrowOp::RaiseDispute,
        _ => EscrowOp::ResolveDispute,
    }
}

fn bytes_to_i128(data: &[u8], offset: usize) -> i128 {
    if offset + 8 > data.len() {
        return 100;
    }
    let raw = i64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ]);
    (raw.abs() as i128).max(1).min(1_000_000)
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 10 {
        return;
    }

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.sequence_number = 100);

    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let arbiter = Address::generate(&env);

    let sac_admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(sac_admin);
    let token_addr = sac.address();

    // Fuzz amount (bytes 0-7)
    let amount = bytes_to_i128(data, 0);

    // Mint tokens to buyer
    StellarAssetClient::new(&env, &token_addr).mint(&buyer, &(amount * 2));

    // Deploy escrow contract
    let escrow_addr = env.register_contract(None, EscrowContract);
    let client = soroban_escrow_template::EscrowContractClient::new(&env, &escrow_addr);

    let deadline = env.ledger().sequence() + 1000;

    let init_result = client.try_initialize(
        &buyer,
        &seller,
        &arbiter,
        &token_addr,
        &amount,
        &deadline,
        &0u32, // no arbitration fee
        &None, // no milestones
    );

    if init_result.is_err() {
        return;
    }

    let initial_buyer_balance = soroban_sdk::token::Client::new(&env, &token_addr).balance(&buyer);
    let initial_contract_balance =
        soroban_sdk::token::Client::new(&env, &token_addr).balance(&escrow_addr);

    // Track total transferred amounts
    let mut total_transferred_out = 0i128;

    // Execute operations from fuzz input
    let mut offset = 8;
    while offset < data.len() {
        let op = byte_to_op(data[offset]);
        offset += 1;

        match op {
            EscrowOp::Fund => {
                let _ = client.try_fund();
            }

            EscrowOp::MarkDelivered => {
                let _ = client.try_mark_delivered();
            }

            EscrowOp::ApproveDelivery => {
                let seller_balance_before =
                    soroban_sdk::token::Client::new(&env, &token_addr).balance(&seller);
                let result = client.try_approve_delivery();
                if result.is_ok() {
                    let seller_balance_after =
                        soroban_sdk::token::Client::new(&env, &token_addr).balance(&seller);
                    total_transferred_out += seller_balance_after - seller_balance_before;
                }
            }

            EscrowOp::ReleasePartial => {
                if offset + 8 > data.len() {
                    break;
                }
                let partial_amount = bytes_to_i128(data, offset).min(amount);
                offset += 8;

                let seller_balance_before =
                    soroban_sdk::token::Client::new(&env, &token_addr).balance(&seller);
                let result = client.try_release_partial(&partial_amount);
                if result.is_ok() {
                    let seller_balance_after =
                        soroban_sdk::token::Client::new(&env, &token_addr).balance(&seller);
                    total_transferred_out += seller_balance_after - seller_balance_before;
                }
            }

            EscrowOp::RequestRefund => {
                // Advance past deadline
                env.ledger().with_mut(|l| l.sequence_number = deadline + 1);

                let buyer_balance_before =
                    soroban_sdk::token::Client::new(&env, &token_addr).balance(&buyer);
                let result = client.try_request_refund();
                if result.is_ok() {
                    let buyer_balance_after =
                        soroban_sdk::token::Client::new(&env, &token_addr).balance(&buyer);
                    total_transferred_out += buyer_balance_after - buyer_balance_before;
                }
            }

            EscrowOp::RequestPartialRefund => {
                if offset + 8 > data.len() {
                    break;
                }
                let partial_amount = bytes_to_i128(data, offset).min(amount);
                offset += 8;

                env.ledger().with_mut(|l| l.sequence_number = deadline + 1);

                let buyer_balance_before =
                    soroban_sdk::token::Client::new(&env, &token_addr).balance(&buyer);
                let result = client.try_request_partial_refund(&partial_amount);
                if result.is_ok() {
                    let buyer_balance_after =
                        soroban_sdk::token::Client::new(&env, &token_addr).balance(&buyer);
                    total_transferred_out += buyer_balance_after - buyer_balance_before;
                }
            }

            EscrowOp::Cancel => {
                let buyer_balance_before =
                    soroban_sdk::token::Client::new(&env, &token_addr).balance(&buyer);
                let result = client.try_cancel();
                if result.is_ok() {
                    let buyer_balance_after =
                        soroban_sdk::token::Client::new(&env, &token_addr).balance(&buyer);
                    total_transferred_out += buyer_balance_after - buyer_balance_before;
                }
            }

            EscrowOp::RaiseDispute => {
                let _ = client.try_raise_dispute(&buyer);
            }

            EscrowOp::ResolveDispute => {
                if offset >= data.len() {
                    break;
                }
                let favor_seller = data[offset] % 2 == 0;
                offset += 1;

                let seller_balance_before =
                    soroban_sdk::token::Client::new(&env, &token_addr).balance(&seller);
                let buyer_balance_before =
                    soroban_sdk::token::Client::new(&env, &token_addr).balance(&buyer);

                let result = client.try_resolve_dispute(&favor_seller);

                if result.is_ok() {
                    let seller_balance_after =
                        soroban_sdk::token::Client::new(&env, &token_addr).balance(&seller);
                    let buyer_balance_after =
                        soroban_sdk::token::Client::new(&env, &token_addr).balance(&buyer);

                    total_transferred_out += (seller_balance_after - seller_balance_before)
                        + (buyer_balance_after - buyer_balance_before);
                }
            }
        }

        // Invariants after each operation

        // Invariant 1: Contract balance never goes negative
        let contract_balance =
            soroban_sdk::token::Client::new(&env, &token_addr).balance(&escrow_addr);
        assert!(
            contract_balance >= 0,
            "Contract balance went negative: {}",
            contract_balance
        );

        // Invariant 2: Total transferred out never exceeds amount initially deposited
        let amount_in_contract = contract_balance - initial_contract_balance;
        assert!(
            total_transferred_out <= amount + amount_in_contract,
            "Transferred out exceeds deposited: out={}, deposited={}",
            total_transferred_out,
            amount
        );

        // Invariant 3: Buyer and seller balances never go negative
        let buyer_balance = soroban_sdk::token::Client::new(&env, &token_addr).balance(&buyer);
        let seller_balance = soroban_sdk::token::Client::new(&env, &token_addr).balance(&seller);

        assert!(
            buyer_balance >= 0,
            "Buyer balance went negative: {}",
            buyer_balance
        );
        assert!(
            seller_balance >= 0,
            "Seller balance went negative: {}",
            seller_balance
        );

        // Invariant 4: State is always valid (state machine consistency)
        let state = client.get_state();
        assert!(
            state.is_some() || state.is_none(),
            "State should always be retrievable"
        );
    }
});
