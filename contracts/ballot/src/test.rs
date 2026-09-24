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

    client.tally();

    // After tally, voting is closed — new votes should fail
    let voter2 = Address::generate(&env);
    client.register_voter(&voter2);
    env.ledger()
        .with_mut(|l| l.sequence_number = VOTING_START + 1);
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

/// Vote exactly at voting_end is accepted.
#[test]
fn test_vote_at_window_end_accepted() {
    let env = make_env();
    let (client, _admin) = setup(&env);
    let voter = Address::generate(&env);

    client.register_voter(&voter);
    env.ledger().with_mut(|le| le.sequence_number = VOTING_END);
    client.vote(&voter, &1u32);
    assert_eq!(client.get_yes_votes(), 1);
}

/// initialize with invalid window (start >= end) is rejected (#9).
#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_invalid_window_rejected() {
    let env = make_env();
    let admin = Address::generate(&env);
    let addr = env.register_contract(None, BallotContract);
    let client = BallotContractClient::new(&env, &addr);
    let choices = vec![
        &env,
        String::from_str(&env, "no"),
        String::from_str(&env, "yes"),
    ];
    // start == end → InvalidWindow
    client.initialize(&admin, &50u32, &50u32, &choices);
}

// ---------------------------------------------------------------------------
// Issue #786 — deregister_voter tests
// ---------------------------------------------------------------------------

/// Admin can deregister a voter before any vote has been cast.
#[test]
fn test_deregister_voter_before_vote() {
    let env = make_env();
    let (client, _admin) = setup(&env);
    let voter = Address::generate(&env);

    client.register_voter(&voter);
    client.deregister_voter(&voter);

    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_START);
    let result = client.try_vote(&voter, &1u32);
    assert!(result.is_err());
}

/// deregister_voter is rejected once a vote has been cast (#8).
#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_deregister_voter_after_vote_rejected() {
    let env = make_env();
    let (client, _admin) = setup(&env);
    let voter1 = Address::generate(&env);
    let voter2 = Address::generate(&env);

    client.register_voter(&voter1);
    client.register_voter(&voter2);

    env.ledger()
        .with_mut(|le| le.sequence_number = VOTING_START);
    client.vote(&voter1, &1u32);

    // Should fail with VotingAlreadyStarted (#8)
    client.deregister_voter(&voter2);
}

/// deregister_voter on a non-registered voter fails with NotRegistered (#4).
#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_deregister_unregistered_voter_rejected() {
    let env = make_env();
    let (client, _admin) = setup(&env);
    let non_voter = Address::generate(&env);

    client.deregister_voter(&non_voter);
}

// ---------------------------------------------------------------------------
// Issue #788 — Multi-choice ballot tests
// ---------------------------------------------------------------------------

/// Helper: 3-choice ballot ("red", "green", "blue").
fn setup_multi(env: &Env) -> (BallotContractClient, Address) {
    let admin = Address::generate(env);
    let addr = env.register_contract(None, BallotContract);
    let client = BallotContractClient::new(env, &addr);
    let choices = vec![
        env,
        String::from_str(env, "red"),
        String::from_str(env, "green"),
        String::from_str(env, "blue"),
    ];
    client.initialize(&admin, &VOTING_START, &VOTING_END, &choices);
    (client, admin)
}

/// tally_all returns per-choice counts for a 3-choice ballot.
#[test]
fn test_multi_choice_tally_all_returns_per_choice_counts() {
    let env = make_env();
    let (client, _admin) = setup_multi(&env);

    let v0 = Address::generate(&env);
    let v1 = Address::generate(&env);
    let v2 = Address::generate(&env);
    let v3 = Address::generate(&env);

    client.register_voter(&v0);
    client.register_voter(&v1);
    client.register_voter(&v2);
    client.register_voter(&v3);

    env.ledger().with_mut(|l| l.sequence_number = VOTING_START);

    // v0 → red (0), v1 → green (1), v2 → green (1), v3 → blue (2)
    client.vote(&v0, &0u32);
    client.vote(&v1, &1u32);
    client.vote(&v2, &1u32);
    client.vote(&v3, &2u32);

    let counts = client.tally_all();
    assert_eq!(counts.len(), 3);
    assert_eq!(counts.get(0).unwrap(), 1); // red
    assert_eq!(counts.get(1).unwrap(), 2); // green
    assert_eq!(counts.get(2).unwrap(), 1); // blue
}

/// get_choice_votes returns the correct tally for a specific index.
#[test]
fn test_multi_choice_get_choice_votes() {
    let env = make_env();
    let (client, _admin) = setup_multi(&env);

    let voter = Address::generate(&env);
    client.register_voter(&voter);
    env.ledger().with_mut(|l| l.sequence_number = VOTING_START);
    client.vote(&voter, &2u32); // blue

    assert_eq!(client.get_choice_votes(&2u32), 1);
    assert_eq!(client.get_choice_votes(&0u32), 0);
    assert_eq!(client.get_choice_votes(&1u32), 0);
}

/// Out-of-range choice index is rejected (#6).
#[test]
fn test_multi_choice_invalid_index_rejected() {
    let env = make_env();
    let (client, _admin) = setup_multi(&env); // 3 choices: 0,1,2

    let voter = Address::generate(&env);
    client.register_voter(&voter);
    env.ledger().with_mut(|l| l.sequence_number = VOTING_START);

    // Index 3 is out of range.
    let result = client.try_vote(&voter, &3u32);
    assert!(result.is_err());
}

/// tally_all closes voting so further votes are rejected.
#[test]
fn test_multi_choice_tally_all_closes_voting() {
    let env = make_env();
    let (client, _admin) = setup_multi(&env);

    let voter = Address::generate(&env);
    client.register_voter(&voter);
    env.ledger().with_mut(|l| l.sequence_number = VOTING_START);
    client.vote(&voter, &1u32);
    client.tally_all();

    let voter2 = Address::generate(&env);
    client.register_voter(&voter2);
    let result = client.try_vote(&voter2, &0u32);
    assert!(result.is_err(), "voting should be closed after tally_all");
}

/// get_choices returns labels in declaration order.
#[test]
fn test_get_choices_returns_labels() {
    let env = make_env();
    let (client, _admin) = setup_multi(&env);

    let choices = client.get_choices();
    assert_eq!(choices.len(), 3);
    assert_eq!(choices.get(0).unwrap(), String::from_str(&env, "red"));
    assert_eq!(choices.get(1).unwrap(), String::from_str(&env, "green"));
    assert_eq!(choices.get(2).unwrap(), String::from_str(&env, "blue"));
}

/// Empty choices list is rejected (#11).
#[test]
fn test_no_choices_rejected() {
    let env = make_env();
    let admin = Address::generate(&env);
    let addr = env.register_contract(None, BallotContract);
    let client = BallotContractClient::new(&env, &addr);
    let empty: soroban_sdk::Vec<String> = soroban_sdk::Vec::new(&env);
    let result = client.try_initialize(&admin, &VOTING_START, &VOTING_END, &empty);
    assert!(result.is_err());
}

/// Five-choice ballot — all five choices receive votes and tally_all is correct.
#[test]
fn test_five_choice_ballot() {
    let env = make_env();
    let admin = Address::generate(&env);
    let addr = env.register_contract(None, BallotContract);
    let client = BallotContractClient::new(&env, &addr);
    let choices = vec![
        &env,
        String::from_str(&env, "a"),
        String::from_str(&env, "b"),
        String::from_str(&env, "c"),
        String::from_str(&env, "d"),
        String::from_str(&env, "e"),
    ];
    client.initialize(&admin, &VOTING_START, &VOTING_END, &choices);

    for i in 0u32..5 {
        let voter = Address::generate(&env);
        client.register_voter(&voter);
        env.ledger().with_mut(|l| l.sequence_number = VOTING_START);
        client.vote(&voter, &i);
    }

    let counts = client.tally_all();
    assert_eq!(counts.len(), 5);
    for i in 0..5 {
        assert_eq!(counts.get(i).unwrap(), 1);
    }
}
