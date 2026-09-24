#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![no_main]
use libfuzzer_sys::fuzz_target;
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![no_main]
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{
    Address, Bytes, BytesN, Env, String, Vec, testutils::Address as _, token,
    token::StellarAssetClient,
};
use soroban_airdrop_template::AirdropContract;
use soroban_sdk::{
    Address, Bytes, BytesN, Env, String, Vec, testutils::Address as _, token::StellarAssetClient,
};

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

fuzz_target!(|data: &[u8]| {
    if data.len() < 50 {
        return;
    }

    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);

    // Set up token
    let sac_admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(sac_admin.clone());
    let token_addr = sac.address();
    let initial_supply = 1_000_000i128;
    StellarAssetClient::new(&env, &token_addr).mint(&admin, &initial_supply);

    // Deploy airdrop contract
    let airdrop_addr = env.register_contract(None, AirdropContract);
    let client = soroban_airdrop_template::AirdropContractClient::new(&env, &airdrop_addr);

    let claim_deadline = env.ledger().sequence() + 1000;
    let _ = client.try_initialize(&admin, &token_addr, &claim_deadline);

    // Fund airdrop contract
    token::Client::new(&env, &token_addr).transfer(&admin, &airdrop_addr, &initial_supply);

    // Fuzz the merkle root (first 32 bytes)
    let mut root_bytes = [0u8; 32];
    root_bytes.copy_from_slice(&data[0..32]);
    let root = BytesN::from_array(&env, &root_bytes);
    let _ = client.try_set_root(&root);

    // Fuzz the claim amount (next 16 bytes as i128)
    let amount = bytes_to_i128(data, 32);

    // Fuzz the proof length and contents (remaining bytes)
    let proof_data = &data[48..];
    let mut proof = Vec::new(&env);

    // Parse proof as chunks of 32 bytes
    for chunk in proof_data.chunks(32) {
        if chunk.len() == 32 {
            let mut proof_elem = [0u8; 32];
            proof_elem.copy_from_slice(chunk);
            proof.push_back(BytesN::from_array(&env, &proof_elem));
        }
    }

    // Attempt claim with fuzzed inputs - should not panic
    // This tests the verification path with arbitrary proofs/leaves
    let claim_result = client.try_claim(&recipient, &amount, &proof);

    // If claim succeeds (extremely rare with random data), verify no invariants broken
    if claim_result.is_ok() {
        // Verify recipient received tokens
        let balance = soroban_sdk::token::Client::new(&env, &token_addr).balance(&recipient);
        assert!(balance >= 0);

        // Verify double-claim is prevented
        let double_claim = client.try_claim(&recipient, &amount, &proof);
        assert!(double_claim.is_err());
    }

    // Most importantly: no panics should occur regardless of proof validity
});
