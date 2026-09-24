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
    Address, Env, String,
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
    vec,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Standard test setup: voting_period=100, quorum=500, no bond, adaptive quorum disabled.
fn setup(env: &Env) -> (DaoContractClient, Address, Address, Address) {
    let admin = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token = sac.address();

    let addr = env.register_contract(None, DaoContract);
    let client = DaoContractClient::new(env, &addr);
    // proposal_bond=0, min/max_quorum_bps=0 → adaptive quorum disabled
    client.initialize(&admin, &token, &100, &500, &0, &0, &0);

    (client, admin, token, addr)
}

/// Setup with a non-zero proposal bond.
fn setup_with_bond(env: &Env, bond: i128) -> (DaoContractClient, Address, Address, Address) {
    let admin = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token = sac.address();

    let addr = env.register_contract(None, DaoContract);
    let client = DaoContractClient::new(env, &addr);
    client.initialize(&admin, &token, &100, &500, &bond, &0, &0);

    (client, admin, token, addr)
}

/// Setup with adaptive quorum enabled.
fn setup_adaptive(env: &Env, min_bps: u32, max_bps: u32) -> (DaoContractClient, Address, Address, Address) {
    let admin = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token = sac.address();

    let addr = env.register_contract(None, DaoContract);
    let client = DaoContractClient::new(env, &addr);
    client.initialize(&admin, &token, &100, &500, &0, &min_bps, &max_bps);

    (client, admin, token, addr)
}

fn mint_tokens(env: &Env, token: &Address, admin: &Address, to: &Address, amount: i128) {
    StellarAssetClient::new(env, token).mint(to, &amount);
    let _ = admin;
}

/// Helper: create a plain proposal (no action payload).
fn create_plain_proposal(
    client: &DaoContractClient,
    env: &Env,
    proposer: &Address,
) -> u32 {
    client.create_proposal(
        proposer,
        &String::from_str(env, "P"),
        &String::from_str(env, "D"),
        &None,
        &None,
        &None,
    )
}

// ---------------------------------------------------------------------------
// Core lifecycle tests
// ---------------------------------------------------------------------------

#[test]
fn test_initialize() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _, _, _) = setup(&env);
    assert_eq!(client.proposal_count(), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_initialize_twice_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);
    client.initialize(&admin, &token, &100, &500, &0, &0, &0);
}

#[test]
fn test_create_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);

    mint_tokens(&env, &token, &admin, &admin, 1_000);

    let id = client.create_proposal(
        &admin,
        &String::from_str(&env, "Upgrade Protocol"),
        &String::from_str(&env, "Upgrade to v2"),
        &None,
        &None,
        &None,
    );
    assert_eq!(id, 0);
    assert_eq!(client.proposal_count(), 1);

    let proposal = client.get_proposal(&0);
    assert_eq!(proposal.state, ProposalState::Active);
    assert_eq!(proposal.yes_votes, 0);
    assert_eq!(proposal.no_votes, 0);
    assert_eq!(proposal.bond_amount, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_create_proposal_no_tokens_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _, _, _) = setup(&env);

    let proposer = Address::generate(&env);
    client.create_proposal(
        &proposer,
        &String::from_str(&env, "Bad Proposal"),
        &String::from_str(&env, "no tokens"),
        &None,
        &None,
        &None,
    );
}

#[test]
fn test_vote_yes() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);

    mint_tokens(&env, &token, &admin, &admin, 1_000);
    let id = create_plain_proposal(&client, &env, &admin);

    let voter = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &voter, 600);
    client.vote(&voter, &id, &true);

    let proposal = client.get_proposal(&id);
    assert_eq!(proposal.yes_votes, 600);
    assert_eq!(proposal.no_votes, 0);
}

#[test]
fn test_vote_no() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);

    mint_tokens(&env, &token, &admin, &admin, 1_000);
    let id = create_plain_proposal(&client, &env, &admin);

    let voter = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &voter, 300);
    client.vote(&voter, &id, &false);

    let proposal = client.get_proposal(&id);
    assert_eq!(proposal.yes_votes, 0);
    assert_eq!(proposal.no_votes, 300);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_vote_twice_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);

    mint_tokens(&env, &token, &admin, &admin, 1_000);
    let id = create_plain_proposal(&client, &env, &admin);

    let voter = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &voter, 100);
    client.vote(&voter, &id, &true);
    client.vote(&voter, &id, &true);
}

#[test]
fn test_execute_proposal_passes() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);

    mint_tokens(&env, &token, &admin, &admin, 1_000);
    let id = create_plain_proposal(&client, &env, &admin);

    let voter = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &voter, 600);
    client.vote(&voter, &id, &true);

    let deadline = client.get_proposal(&id).deadline;
    env.ledger().with_mut(|l| l.sequence_number = deadline + 1);

    client.execute_proposal(&id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Executed);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_execute_before_deadline_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);

    mint_tokens(&env, &token, &admin, &admin, 1_000);
    let id = create_plain_proposal(&client, &env, &admin);

    let voter = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &voter, 600);
    client.vote(&voter, &id, &true);
    client.execute_proposal(&id);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_execute_quorum_not_met_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);

    mint_tokens(&env, &token, &admin, &admin, 1_000);
    let id = create_plain_proposal(&client, &env, &admin);

    let voter = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &voter, 100); // 100 < 500 quorum
    client.vote(&voter, &id, &true);

    let deadline = client.get_proposal(&id).deadline;
    env.ledger().with_mut(|l| l.sequence_number = deadline + 1);
    client.execute_proposal(&id);
}

#[test]
fn test_cancel_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);

    mint_tokens(&env, &token, &admin, &admin, 1_000);
    let id = create_plain_proposal(&client, &env, &admin);

    client.cancel_proposal(&id);
    assert_eq!(client.get_proposal(&id).state, ProposalState::Cancelled);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_cancel_already_executed_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);

    mint_tokens(&env, &token, &admin, &admin, 1_000);
    let id = create_plain_proposal(&client, &env, &admin);

    let voter = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &voter, 600);
    client.vote(&voter, &id, &true);

    let deadline = client.get_proposal(&id).deadline;
    env.ledger().with_mut(|l| l.sequence_number = deadline + 1);
    client.execute_proposal(&id);

    client.cancel_proposal(&id);
}

// ---------------------------------------------------------------------------
// Issue #1106 — Proposal bond tests
// ---------------------------------------------------------------------------

#[test]
fn test_bond_escrowed_on_create() {
    let env = Env::default();
    env.mock_all_auths();
    let bond = 200_i128;
    let (client, admin, token, dao_addr) = setup_with_bond(&env, bond);

    mint_tokens(&env, &token, &admin, &admin, 1_000);

    let before_dao = soroban_sdk::token::Client::new(&env, &token).balance(&dao_addr);
    create_plain_proposal(&client, &env, &admin);
    let after_dao = soroban_sdk::token::Client::new(&env, &token).balance(&dao_addr);

    assert_eq!(after_dao - before_dao, bond, "bond should be held by DAO");
    let proposal = client.get_proposal(&0);
    assert_eq!(proposal.bond_amount, bond);
}

#[test]
fn test_bond_refunded_on_execution() {
    let env = Env::default();
    env.mock_all_auths();
    let bond = 200_i128;
    let (client, admin, token, _) = setup_with_bond(&env, bond);

    mint_tokens(&env, &token, &admin, &admin, 2_000);

    let id = create_plain_proposal(&client, &env, &admin);

    let voter = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &voter, 600);
    client.vote(&voter, &id, &true);

    let deadline = client.get_proposal(&id).deadline;
    env.ledger().with_mut(|l| l.sequence_number = deadline + 1);

    let before = soroban_sdk::token::Client::new(&env, &token).balance(&admin);
    client.execute_proposal(&id);
    let after = soroban_sdk::token::Client::new(&env, &token).balance(&admin);

    assert_eq!(after - before, bond, "bond should be refunded to proposer");
}

#[test]
fn test_bond_slashed_on_quorum_failure() {
    let env = Env::default();
    env.mock_all_auths();
    let bond = 200_i128;
    let (client, admin, token, _) = setup_with_bond(&env, bond);

    mint_tokens(&env, &token, &admin, &admin, 2_000);
    let id = create_plain_proposal(&client, &env, &admin);

    // Vote only 100 — below quorum of 500.
    let voter = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &voter, 100);
    client.vote(&voter, &id, &true);

    let deadline = client.get_proposal(&id).deadline;
    env.ledger().with_mut(|l| l.sequence_number = deadline + 1);

    let before_admin = soroban_sdk::token::Client::new(&env, &token).balance(&admin);
    // execute_proposal returns QuorumNotMet but still slashes the bond.
    let result = client.try_execute_proposal(&id);
    assert!(result.is_err(), "should fail with QuorumNotMet");

    let after_admin = soroban_sdk::token::Client::new(&env, &token).balance(&admin);
    assert_eq!(after_admin - before_admin, bond, "bond should be slashed to admin treasury");
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_create_proposal_insufficient_bond_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let bond = 500_i128;
    let (client, admin, token, _) = setup_with_bond(&env, bond);

    // Mint only 100 — less than the 500-token bond requirement.
    mint_tokens(&env, &token, &admin, &admin, 100);
    create_plain_proposal(&client, &env, &admin);
}

// ---------------------------------------------------------------------------
// Issue #1108 — Executable action payload tests
// ---------------------------------------------------------------------------

#[test]
fn test_create_proposal_with_action_payload_stored() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);
    mint_tokens(&env, &token, &admin, &admin, 1_000);

    // Register a dummy target contract just to capture the address.
    let target = Address::generate(&env);
    let function = Symbol::new(&env, "transfer");
    let args: soroban_sdk::Vec<soroban_sdk::Val> = vec![&env];

    let id = client.create_proposal(
        &admin,
        &String::from_str(&env, "Fund transfer"),
        &String::from_str(&env, "Send tokens"),
        &Some(target.clone()),
        &Some(function.clone()),
        &Some(args.clone()),
    );

    let proposal = client.get_proposal(&id);
    assert_eq!(proposal.action_target, Some(target));
    assert_eq!(proposal.action_function, Some(function));
    assert!(proposal.action_args.is_some());
}

// ---------------------------------------------------------------------------
// Issue #1107 — Adaptive quorum tests
// ---------------------------------------------------------------------------

#[test]
fn test_adaptive_quorum_updates_after_execution() {
    let env = Env::default();
    env.mock_all_auths();
    // Enable adaptive quorum: 1000–9000 bps (10%–90%)
    let (client, admin, token, _) = setup_adaptive(&env, 1_000, 9_000);

    mint_tokens(&env, &token, &admin, &admin, 2_000);

    let initial_ema = client.current_quorum_bps();

    let id = create_plain_proposal(&client, &env, &admin);
    let voter = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &voter, 600);
    client.vote(&voter, &id, &true);

    let deadline = client.get_proposal(&id).deadline;
    env.ledger().with_mut(|l| l.sequence_number = deadline + 1);
    client.execute_proposal(&id);

    let updated_ema = client.current_quorum_bps();
    // EMA must change and remain within bounds.
    assert!(updated_ema >= 1_000, "ema below min");
    assert!(updated_ema <= 9_000, "ema above max");
    // With quorum=500 and total_votes=600, participation BPS = 10_000; EMA
    // should have moved upward from the midpoint of 5000.
    assert_ne!(updated_ema, initial_ema, "EMA should have changed");
}

#[test]
fn test_adaptive_quorum_disabled_when_bounds_zero() {
    let env = Env::default();
    env.mock_all_auths();
    // Both bounds zero → adaptive quorum disabled
    let (client, admin, token, _) = setup(&env);
    mint_tokens(&env, &token, &admin, &admin, 2_000);

    let initial_ema = client.current_quorum_bps();
    assert_eq!(initial_ema, 0, "EMA should start at 0 when disabled");

    let id = create_plain_proposal(&client, &env, &admin);
    let voter = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &voter, 600);
    client.vote(&voter, &id, &true);

    let deadline = client.get_proposal(&id).deadline;
    env.ledger().with_mut(|l| l.sequence_number = deadline + 1);
    client.execute_proposal(&id);

    assert_eq!(
        client.current_quorum_bps(),
        0,
        "EMA should remain 0 when adaptive quorum is disabled"
    );
}

#[test]
fn test_adaptive_quorum_clamped_to_min() {
    let env = Env::default();
    env.mock_all_auths();
    // Narrow band: 4000–6000 bps
    let (client, admin, token, _) = setup_adaptive(&env, 4_000, 6_000);
    mint_tokens(&env, &token, &admin, &admin, 2_000);

    // Run several low-participation proposals to push EMA toward min.
    for _ in 0..5u32 {
        let id = create_plain_proposal(&client, &env, &admin);
        // Vote exactly at quorum (500) → 100% participation BPS = 10_000.
        // Actually use a voter with just above quorum to test clamping.
        // Use a tiny voter (participation BPS = 0) — quorum not met path.
        let deadline = client.get_proposal(&id).deadline;
        env.ledger().with_mut(|l| l.sequence_number = deadline + 1);
        // Don't vote — quorum will not be met, slash path, EMA gets 0.
        let _ = client.try_execute_proposal(&id);
    }

    let ema = client.current_quorum_bps();
    assert!(ema >= 4_000, "EMA must not go below min_quorum_bps");
    assert!(ema <= 6_000, "EMA must not exceed max_quorum_bps");
}

#[test]
fn test_total_votes_invariant() {
    // yes_votes + no_votes == total_votes always holds.
    let env = Env::default();
    env.mock_all_auths();
    let (client, admin, token, _) = setup(&env);

    mint_tokens(&env, &token, &admin, &admin, 1_000);
    let id = create_plain_proposal(&client, &env, &admin);

    let v1 = Address::generate(&env);
    let v2 = Address::generate(&env);
    mint_tokens(&env, &token, &admin, &v1, 300);
    mint_tokens(&env, &token, &admin, &v2, 400);

    client.vote(&v1, &id, &true);
    client.vote(&v2, &id, &false);

    let p = client.get_proposal(&id);
    assert_eq!(p.yes_votes + p.no_votes, 700);
}
