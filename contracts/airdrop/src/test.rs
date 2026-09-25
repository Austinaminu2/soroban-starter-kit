#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use super::*;
use soroban_sdk::{
    Address, Bytes, BytesN, Env, Vec,
    testutils::{Address as _, Ledger as _},
    token::{Client as TokenClient, StellarAssetClient},
    xdr::ToXdr,
};

// ---------------------------------------------------------------------------
// Merkle tree helpers (replicates on-chain logic for test setup)
// ---------------------------------------------------------------------------

fn sha256(env: &Env, data: &Bytes) -> BytesN<32> {
    env.crypto().sha256(data).into()
}

fn leaf(env: &Env, recipient: &Address, amount: i128) -> BytesN<32> {
    let mut data = Bytes::new(env);
    data.append(&recipient.clone().to_xdr(env));
    let amount_bytes: [u8; 16] = amount.to_be_bytes();
    data.append(&Bytes::from_slice(env, &amount_bytes));
    sha256(env, &data)
}

fn hash_pair(env: &Env, a: &BytesN<32>, b: &BytesN<32>) -> BytesN<32> {
    let mut data = Bytes::new(env);
    if a.to_array() <= b.to_array() {
        data.append(&Bytes::from(a.clone()));
        data.append(&Bytes::from(b.clone()));
    } else {
        data.append(&Bytes::from(b.clone()));
        data.append(&Bytes::from(a.clone()));
    }
    sha256(env, &data)
}

/// Build a two-leaf tree. Returns (root, proof_for_leaf_0, proof_for_leaf_1).
fn two_leaf_tree(
    env: &Env,
    leaf0: BytesN<32>,
    leaf1: BytesN<32>,
) -> (BytesN<32>, Vec<BytesN<32>>, Vec<BytesN<32>>) {
    let root = hash_pair(env, &leaf0, &leaf1);
    let mut proof0 = Vec::new(env);
    proof0.push_back(leaf1.clone());
    let mut proof1 = Vec::new(env);
    proof1.push_back(leaf0.clone());
    (root, proof0, proof1)
}

// ---------------------------------------------------------------------------
// Setup — now passes a claim_deadline far in the future by default
// ---------------------------------------------------------------------------

const FAR_DEADLINE: u32 = 1_000_000;

struct TestEnv<'a> {
    env: Env,
    client: AirdropContractClient<'a>,
    token: Address,
    admin: Address,
    alice: Address,
    bob: Address,
}

fn setup<'a>(env: &'a Env) -> TestEnv<'a> {
    env.mock_all_auths();

    let admin = Address::generate(env);
    let alice = Address::generate(env);
    let bob = Address::generate(env);

    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let airdrop = env.register_contract(None, AirdropContract);
    let client = AirdropContractClient::new(env, &airdrop);
    client.initialize(&admin, &token, &FAR_DEADLINE);

    // Fund the airdrop contract with tokens
    StellarAssetClient::new(env, &token).mint(&airdrop, &100_000i128);

    TestEnv {
        env: env.clone(),
        client,
        token,
        admin,
        alice,
        bob,
    }
}

// ---------------------------------------------------------------------------
// Existing tests (updated for new initialize signature)
// ---------------------------------------------------------------------------

#[test]
fn test_initialize_rejects_duplicate() {
    let env = Env::default();
    let t = setup(&env);
    let res = t.client.try_initialize(&t.admin, &t.token, &FAR_DEADLINE);
    assert!(res.is_err());
}

#[test]
fn test_claim_happy_path() {
    let env = Env::default();
    let t = setup(&env);

    let alice_amount = 1_000i128;
    let bob_amount = 2_000i128;

    let leaf_a = leaf(&env, &t.alice, alice_amount);
    let leaf_b = leaf(&env, &t.bob, bob_amount);
    let (root, proof_a, _) = two_leaf_tree(&env, leaf_a, leaf_b);

    t.client.set_root(&1u32, &root);

    let before = TokenClient::new(&env, &t.token).balance(&t.alice);
    t.client.claim(&1u32, &t.alice, &alice_amount, &proof_a);
    assert_eq!(
        TokenClient::new(&env, &t.token).balance(&t.alice),
        before + alice_amount
    );
    assert!(t.client.is_claimed(&1u32, &t.alice));
}

#[test]
fn test_duplicate_claim_rejected() {
    let env = Env::default();
    let t = setup(&env);

    let alice_amount = 500i128;
    let bob_amount = 500i128;

    let leaf_a = leaf(&env, &t.alice, alice_amount);
    let leaf_b = leaf(&env, &t.bob, bob_amount);
    let (root, proof_a, _) = two_leaf_tree(&env, leaf_a, leaf_b);

    t.client.set_root(&1u32, &root);
    t.client.claim(&1u32, &t.alice, &alice_amount, &proof_a);

    let res = t.client.try_claim(&1u32, &t.alice, &alice_amount, &proof_a);
    assert!(res.is_err());
}

#[test]
fn test_invalid_proof_rejected() {
    let env = Env::default();
    let t = setup(&env);

    let alice_amount = 1_000i128;
    let bob_amount = 2_000i128;

    let leaf_a = leaf(&env, &t.alice, alice_amount);
    let leaf_b = leaf(&env, &t.bob, bob_amount);
    let (root, _proof_a, proof_b) = two_leaf_tree(&env, leaf_a, leaf_b);

    t.client.set_root(&1u32, &root);

    // Bob's proof used for Alice's claim — must fail
    let res = t.client.try_claim(&1u32, &t.alice, &alice_amount, &proof_b);
    assert!(res.is_err());
}

#[test]
fn test_wrong_amount_rejected() {
    let env = Env::default();
    let t = setup(&env);

    let alice_amount = 1_000i128;
    let bob_amount = 2_000i128;

    let leaf_a = leaf(&env, &t.alice, alice_amount);
    let leaf_b = leaf(&env, &t.bob, bob_amount);
    let (root, proof_a, _) = two_leaf_tree(&env, leaf_a, leaf_b);

    t.client.set_root(&1u32, &root);

    // Wrong amount
    let res = t.client.try_claim(&1u32, &t.alice, &999i128, &proof_a);
    assert!(res.is_err());
}

#[test]
fn test_zero_amount_rejected() {
    let env = Env::default();
    let t = setup(&env);
    let root = BytesN::from_array(&env, &[0u8; 32]);
    t.client.set_root(&1u32, &root);
    let proof = Vec::new(&env);
    let res = t.client.try_claim(&1u32, &t.alice, &0i128, &proof);
    assert!(res.is_err());
}

#[test]
fn test_claim_without_root_fails() {
    let env = Env::default();
    let t = setup(&env);
    let proof = Vec::new(&env);
    let res = t.client.try_claim(&1u32, &t.alice, &1_000i128, &proof);
    assert!(res.is_err());
}

#[test]
fn test_both_recipients_claim() {
    let env = Env::default();
    let t = setup(&env);

    let alice_amount = 300i128;
    let bob_amount = 700i128;

    let leaf_a = leaf(&env, &t.alice, alice_amount);
    let leaf_b = leaf(&env, &t.bob, bob_amount);
    let (root, proof_a, proof_b) = two_leaf_tree(&env, leaf_a, leaf_b);

    t.client.set_root(&1u32, &root);
    t.client.claim(&1u32, &t.alice, &alice_amount, &proof_a);
    t.client.claim(&1u32, &t.bob, &bob_amount, &proof_b);

    assert_eq!(
        TokenClient::new(&env, &t.token).balance(&t.alice),
        alice_amount
    );
    assert_eq!(TokenClient::new(&env, &t.token).balance(&t.bob), bob_amount);
}

// ---------------------------------------------------------------------------
// Claim deadline tests — #780
// ---------------------------------------------------------------------------

/// Claim succeeds when ledger sequence < deadline.
#[test]
fn test_claim_before_deadline_succeeds() {
    let env = Env::default();
    env.mock_all_auths();
    // Start at ledger 100; deadline = 200
    env.ledger().with_mut(|l| l.sequence_number = 100);

    let admin = Address::generate(&env);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let airdrop = env.register_contract(None, AirdropContract);
    let client = AirdropContractClient::new(&env, &airdrop);
    client.initialize(&admin, &token, &200u32);
    StellarAssetClient::new(&env, &token).mint(&airdrop, &10_000i128);

    let leaf_a = leaf(&env, &alice, 500i128);
    let leaf_b = leaf(&env, &bob, 500i128);
    let (root, proof_a, _) = two_leaf_tree(&env, leaf_a, leaf_b);
    client.set_root(&1u32, &root);

    // ledger 100 < 200 — should succeed
    client.claim(&1u32, &alice, &500i128, &proof_a);
    assert!(client.is_claimed(&1u32, &alice));
}

// ---------------------------------------------------------------------------
// Multi-round tests — #1148
// ---------------------------------------------------------------------------

/// A recipient who claimed in round 1 can still claim in round 2.
#[test]
fn test_multi_round_claim_independence() {
    let env = Env::default();
    let t = setup(&env);

    // Round 1 tree
    let r1_alice = 1_000i128;
    let r1_bob = 2_000i128;
    let leaf_a1 = leaf(&env, &t.alice, r1_alice);
    let leaf_b1 = leaf(&env, &t.bob, r1_bob);
    let (root1, proof_a1, _) = two_leaf_tree(&env, leaf_a1, leaf_b1);

    t.client.set_root(&1u32, &root1);
    t.client.claim(&1u32, &t.alice, &r1_alice, &proof_a1);
    assert!(t.client.is_claimed(&1u32, &t.alice));

    // Round 2 tree — Alice claims again with a fresh allocation
    let r2_alice = 500i128;
    let r2_bob = 500i128;
    let leaf_a2 = leaf(&env, &t.alice, r2_alice);
    let leaf_b2 = leaf(&env, &t.bob, r2_bob);
    let (root2, proof_a2, _) = two_leaf_tree(&env, leaf_a2, leaf_b2);

    t.client.set_root(&2u32, &root2);
    t.client.claim(&2u32, &t.alice, &r2_alice, &proof_a2);

    // Round 1 record is untouched; round 2 record is independent.
    assert!(t.client.is_claimed(&1u32, &t.alice));
    assert!(t.client.is_claimed(&2u32, &t.alice));
    assert_eq!(
        TokenClient::new(&env, &t.token).balance(&t.alice),
        r1_alice + r2_alice
    );
}

/// A round-1 proof cannot be replayed against round 2's root.
#[test]
fn test_round_proofs_are_isolated() {
    let env = Env::default();
    let t = setup(&env);

    let leaf_a1 = leaf(&env, &t.alice, 1_000i128);
    let leaf_b1 = leaf(&env, &t.bob, 2_000i128);
    let (root1, proof_a1, _) = two_leaf_tree(&env, leaf_a1, leaf_b1);
    t.client.set_root(&1u32, &root1);

    let leaf_a2 = leaf(&env, &t.alice, 500i128);
    let leaf_b2 = leaf(&env, &t.bob, 500i128);
    let (root2, _, _) = two_leaf_tree(&env, leaf_a2, leaf_b2);
    t.client.set_root(&2u32, &root2);

    // Round 1 proof against round 2 root must fail.
    let res = t.client.try_claim(&2u32, &t.alice, &1_000i128, &proof_a1);
    assert!(res.is_err());
}

/// Changing the root of an active, unexpired round is rejected.
#[test]
fn test_set_root_rejects_active_round_replacement() {
    let env = Env::default();
    let t = setup(&env);

    let leaf_a = leaf(&env, &t.alice, 1_000i128);
    let leaf_b = leaf(&env, &t.bob, 2_000i128);
    let (root, _, _) = two_leaf_tree(&env, leaf_a, leaf_b);
    t.client.set_root(&1u32, &root);

    // Same round, still unexpired — replacement must be rejected.
    let new_root = BytesN::from_array(&env, &[7u8; 32]);
    let res = t.client.try_set_root(&1u32, &new_root);
    assert!(res.is_err());
}

/// A new round id can be opened even while an earlier round is active.
#[test]
fn test_set_root_allows_new_round() {
    let env = Env::default();
    let t = setup(&env);

    let leaf_a = leaf(&env, &t.alice, 1_000i128);
    let leaf_b = leaf(&env, &t.bob, 2_000i128);
    let (root1, _, _) = two_leaf_tree(&env, leaf_a, leaf_b);
    t.client.set_root(&1u32, &root1);

    let leaf_a2 = leaf(&env, &t.alice, 500i128);
    let leaf_b2 = leaf(&env, &t.bob, 500i128);
    let (root2, _, _) = two_leaf_tree(&env, leaf_a2, leaf_b2);
    t.client.set_root(&2u32, &root2);
}
