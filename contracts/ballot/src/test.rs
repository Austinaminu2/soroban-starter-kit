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
    vec,
};

// Helper: ledger sequence when tests start.
const START_LEDGER: u32 = 10;
const VOTING_START: u32 = 20;
const VOTING_END: u32 = 100;

fn make_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|le| {
        le.sequence_number = START_LEDGER;
    });
    env
}

/// Two-choice ballot (backward compat): choices = ["no", "yes"].
fn setup(env: &Env) -> (BallotContractClient, Address) {
    let admin = Address::generate(env);
    let addr = env.register_contract(None, BallotContract);
    let client = BallotContractClient::new(env, &addr);
    let choices = vec![
        env,
        String::from_str(env, "no"),
        String::from_str(env, "yes"),
    ];
    client.initialize(&admin, &VOTING_START, &VOTING_END, &choices);
    (client, admin)
}

// ---------------------------------------------------------------------------
// Basic lifecycle
// ---------------------------------------------------------------------------

#[test]
fn test_ballot_lifecycle() {
    let env = make_env();
    let (client, _admin) = setup(&env);

    let voter1 = Address::generate(&env);
    let voter2 = Address::generate(&env);

    client.register_voter(&voter1);
    client.register_voter(&voter2);

    // Advance into the voting window
    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_START);

    client.vote(&voter1, &1u32); // yes
    client.vote(&voter2, &0u32); // no

    assert_eq!(client.get_yes_votes(), 1);
    assert_eq!(client.get_no_votes(), 1);

    // Advance past the voting window before tallying.
    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_END + 1);

    let (yes, no) = client.tally();
    assert_eq!(yes, 1);
    assert_eq!(no, 1);
}

// ---------------------------------------------------------------------------
// Double-vote prevention
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_double_vote_prevention() {
    let env = make_env();
    let (client, _admin) = setup(&env);

    let voter = Address::generate(&env);
    client.register_voter(&voter);
    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_START);

    client.vote(&voter, &1u32);
    client.vote(&voter, &1u32); // should panic AlreadyVoted (#5)
}

// ---------------------------------------------------------------------------
// Unregistered voter rejected
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_unregistered_voter_rejected() {
    let env = make_env();
    let (client, _admin) = setup(&env);

    let unregistered = Address::generate(&env);
    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_START);
    client.vote(&unregistered, &1u32); // should panic NotRegistered (#4)
}

// ---------------------------------------------------------------------------
// Invalid choice rejected
// ---------------------------------------------------------------------------

#[test]
fn test_invalid_choice_rejected() {
    let env = make_env();
    let (client, _admin) = setup(&env);

    let voter = Address::generate(&env);
    client.register_voter(&voter);
    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_START);

    // Two choices (0,1): index 2 is invalid.
    let result = client.try_vote(&voter, &2u32);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Double-initialize rejected
// ---------------------------------------------------------------------------

#[test]
fn test_double_initialize_rejected() {
    let env = make_env();
    let (client, admin) = setup(&env);
    let choices = vec![
        &env,
        String::from_str(&env, "no"),
        String::from_str(&env, "yes"),
    ];
    let result = client.try_initialize(&admin, &VOTING_START, &VOTING_END, &choices);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// TTL extension
// ---------------------------------------------------------------------------

#[test]
fn test_register_voter_extends_persistent_ttl() {
    let env = make_env();
    let admin = Address::generate(&env);
    let addr = env.register_contract(None, BallotContract);
    let client = BallotContractClient::new(&env, &addr);
    let big_end: u32 = 100_000;
    let choices = vec![
        &env,
        String::from_str(&env, "no"),
        String::from_str(&env, "yes"),
    ];
    client.initialize(&admin, &VOTING_START, &big_end, &choices);

    let voter = Address::generate(&env);
    client.register_voter(&voter);

    env.ledger()
        .with_mut(|l| l.sequence_number = VOTING_START + 10_000);

    let result = client.try_vote(&voter, &1u32);
    assert!(result.is_ok());
}

#[test]
fn test_vote_extends_persistent_ttl() {
    let env = make_env();
    let (client, _admin) = setup(&env);

    let voter = Address::generate(&env);
    client.register_voter(&voter);
    env.ledger().with_mut(|l| l.sequence_number = VOTING_START);
    client.vote(&voter, &1u32);

    env.ledger()
        .with_mut(|l| l.sequence_number = VOTING_START + 10_000);

    let result = client.try_vote(&voter, &1u32);
    assert!(
        result.is_err(),
        "second vote should be rejected even after ledger advance"
    );
}

// ---------------------------------------------------------------------------
// Tally closes voting
// ---------------------------------------------------------------------------

#[test]
fn test_tally_closes_voting() {
    let env = make_env();
    let (client, _admin) = setup(&env);

    let voter = Address::generate(&env);
    client.register_voter(&voter);
    env.ledger().with_mut(|l| l.sequence_number = VOTING_START);
    client.vote(&voter, &1u32);

    // Advance past the voting window before tallying.
    env.ledger()
        .with_mut(|l| l.sequence_number = VOTING_END + 1);

    client.tally();

    // After tally, voting is closed — new votes should fail
    let voter2 = Address::generate(&env);
    client.register_voter(&voter2);
    env.ledger()
        .with_mut(|l| l.sequence_number = VOTING_END + 2);
    let result = client.try_vote(&voter2, &0u32);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Issue #787 — Voting window tests
// ---------------------------------------------------------------------------

/// Vote before voting_start is rejected with VotingNotStarted (#10).
#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_vote_before_window_rejected() {
    let env = make_env();
    let (client, _admin) = setup(&env);
    let voter = Address::generate(&env);

    client.register_voter(&voter);
    // Still at START_LEDGER (10), before VOTING_START (20)
    client.vote(&voter, &1u32);
}

/// Vote after voting_end is rejected with VotingClosed (#7).
#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_vote_after_window_rejected() {
    let env = make_env();
    let (client, _admin) = setup(&env);
    let voter = Address::generate(&env);

    client.register_voter(&voter);
    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_END + 1);
    client.vote(&voter, &1u32);
}

/// Vote exactly at voting_start is accepted.
#[test]
fn test_vote_at_window_start_accepted() {
    let env = make_env();
    let (client, _admin) = setup(&env);
    let voter = Address::generate(&env);

    client.register_voter(&voter);
    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_START);
    client.vote(&voter, &1u32);
    assert_eq!(client.get_yes_votes(), 1);
}

// ---------------------------------------------------------------------------
// Issue #1121 — Permissionless tally after voting window closes
// ---------------------------------------------------------------------------

/// A non-admin caller can tally once the voting window has closed.
#[test]
fn test_non_admin_can_tally_after_deadline() {
    let env = make_env();
    let (client, _admin) = setup(&env);

    let voter = Address::generate(&env);
    client.register_voter(&voter);
    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_START);
    client.vote(&voter, &1u32);

    // Move past the voting window.
    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_END + 1);

    // A random observer (not the admin) closes the ballot.
    let observer = Address::generate(&env);
    let (yes, no) = client.tally_all(&observer);
    assert_eq!(yes, 1);
    assert_eq!(no, 0);
}

/// tally_all is rejected before the voting window closes.
#[test]
fn test_tally_all_before_deadline_rejected() {
    let env = make_env();
    let (client, _admin) = setup(&env);

    let voter = Address::generate(&env);
    client.register_voter(&voter);
    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_START);
    client.vote(&voter, &1u32);

    // Still inside the voting window.
    let observer = Address::generate(&env);
    let result = client.try_tally_all(&observer);
    assert!(result.is_err());
}

/// get_tally is a read-only query that requires no auth and works after closure.
#[test]
fn test_get_tally_is_permissionless_read_only() {
    let env = make_env();
    let (client, _admin) = setup(&env);

    let voter = Address::generate(&env);
    client.register_voter(&voter);
    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_START);
    client.vote(&voter, &1u32);

    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_END + 1);

    // Anyone can read the tally without auth.
    let (yes, no) = client.get_tally();
    assert_eq!(yes, 1);
    assert_eq!(no, 0);
}
