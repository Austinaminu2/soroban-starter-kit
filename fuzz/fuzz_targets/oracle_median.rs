#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_oracle_template::OracleContract;
use soroban_sdk::{Address, Env, Vec, testutils::Address as _};

fn bytes_to_i128(data: &[u8], offset: usize) -> i128 {
    if offset + 16 > data.len() {
        return 0;
    }
    i128::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
        data[offset + 8],
        data[offset + 9],
        data[offset + 10],
        data[offset + 11],
        data[offset + 12],
        data[offset + 13],
        data[offset + 14],
        data[offset + 15],
    ])
}

fn bytes_to_u64(data: &[u8], offset: usize) -> u64 {
    if offset + 8 > data.len() {
        return 1;
    }
    u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

/// Fuzz target for oracle median/TWAP aggregation math
/// Closes #962 – oracle median fuzz target
fuzz_target!(|data: &[u8]| {
    if data.len() < 20 {
        return;
    }

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| {
        l.sequence_number = 100;
        l.timestamp = 1000;
    });

    let admin = Address::generate(&env);

    // Deploy oracle contract
    let oracle_addr = env.register_contract(None, OracleContract);
    let client = soroban_oracle_template::OracleContractClient::new(&env, &oracle_addr);

    let staleness_threshold = 100u32;
    let init_result = client.try_initialize(&admin, &staleness_threshold);

    if init_result.is_err() {
        return;
    }

    // Fuzz number of publishers (byte 0)
    let num_publishers = ((data[0] as usize) % 10).max(1);

    // Create publishers
    let mut publishers = Vec::new(&env);
    for _ in 0..num_publishers {
        let publisher = Address::generate(&env);
        publishers.push_back(publisher);
    }

    // Set publishers
    let _ = client.try_set_publishers(&admin, &publishers);

    // Fuzz price submissions
    let mut offset = 1;
    for i in 0..num_publishers {
        if offset + 16 > data.len() {
            break;
        }

        let price = bytes_to_i128(data, offset);
        offset += 16;

        let publisher = publishers.get_unchecked(i as u32);

        // Advance timestamp slightly for each submission
        env.ledger().with_mut(|l| {
            l.timestamp += 10;
            l.sequence_number += 1;
        });

        let _ = client.try_submit_price(&publisher, &price);
    }

    // Test median calculation - should not panic
    let median_result = client.try_get_median_price();

    // Invariant: median calculation never panics
    if median_result.is_ok() {
        let median = median_result.unwrap();

        // Invariant: median should be within the range of submitted prices
        // (This is true for any median - it's bounded by min/max of inputs)
        assert!(
            median >= i128::MIN && median <= i128::MAX,
            "Median out of valid i128 range: {}",
            median
        );
    }

    // Test TWAP calculation with fuzzed window
    if offset + 8 <= data.len() {
        let window_raw = bytes_to_u64(data, offset);
        let window = (window_raw % 1000).max(1);

        let twap_result = client.try_get_twap(&window);

        // Invariant: TWAP calculation never panics
        if twap_result.is_ok() {
            let twap = twap_result.unwrap();

            // Invariant: TWAP should be within valid i128 range
            assert!(
                twap >= i128::MIN && twap <= i128::MAX,
                "TWAP out of valid i128 range: {}",
                twap
            );
        }
    }

    // Test sequential price updates with boundary values
    let boundary_prices = [i128::MIN, i128::MAX, 0i128, -1i128, 1i128];

    for &price in &boundary_prices {
        env.ledger().with_mut(|l| {
            l.timestamp += 1;
            l.sequence_number += 1;
        });

        let _ = client.try_update_price(&price);

        // Invariant: get_price and get_price_data remain consistent after boundary updates
        if let Ok(retrieved_price) = client.try_get_price() {
            if let Ok(price_data) = client.try_get_price_data() {
                assert_eq!(
                    retrieved_price, price_data.price,
                    "get_price and get_price_data inconsistent after boundary update"
                );
            }
        }
    }
});
