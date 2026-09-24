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
use soroban_sdk::{Address, Env, String, testutils::Address as _};
use soroban_token_template::TokenContract;

fn bytes_to_i128(data: &[u8], offset: usize) -> i128 {
    i128::from_le_bytes([
        data.get(offset).copied().unwrap_or(0),
        data.get(offset + 1).copied().unwrap_or(0),
        data.get(offset + 2).copied().unwrap_or(0),
        data.get(offset + 3).copied().unwrap_or(0),
        data.get(offset + 4).copied().unwrap_or(0),
        data.get(offset + 5).copied().unwrap_or(0),
        data.get(offset + 6).copied().unwrap_or(0),
        data.get(offset + 7).copied().unwrap_or(0),
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ])
}

fn bytes_to_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data.get(offset).copied().unwrap_or(0),
        data.get(offset + 1).copied().unwrap_or(0),
        data.get(offset + 2).copied().unwrap_or(0),
        data.get(offset + 3).copied().unwrap_or(0),
    ])
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 19 {
        return;
    }

    let env = Env::default();
    env.mock_all_auths();

    // Valid token so initialize's decimals() probe never panics on a bad address.
    let token_admin = Address::generate(&env);
    let token_addr = env.register_contract(None, TokenContract);
    let token_client = soroban_token_template::TokenContractClient::new(&env, &token_addr);
    let _ = token_client.try_initialize(
        &token_admin,
        &String::from_str(&env, "Fuzz Token"),
        &String::from_str(&env, "FZZ"),
        &18u32,
        &None,
    );

    let escrow_addr = env.register_contract(None, EscrowContract);
    let escrow = soroban_escrow_template::EscrowContractClient::new(&env, &escrow_addr);

    let pool = [
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    ];
    let admin = Address::generate(&env);
    let buyer = pool[(data[0] as usize) % pool.len()].clone();
    let seller = pool[(data[1] as usize) % pool.len()].clone();
    let arbiter = pool[(data[2] as usize) % pool.len()].clone();

    let amount = bytes_to_i128(data, 3);
    let deadline = bytes_to_u32(data, 11);
    let dispute_timeout_ledgers = bytes_to_u32(data, 15).max(1);

    let _ = escrow.try_initialize(
        &admin,
        &buyer,
        &seller,
        &arbiter,
        &token_addr,
        &amount,
        &deadline,
        &dispute_timeout_ledgers,
        &None,
    );
});
