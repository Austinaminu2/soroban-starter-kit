#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use crate::storage::ProposalState;
use crate::{DaoContract, DaoContractClient};
use proptest::prelude::*;
use soroban_sdk::{Address, Env, String, testutils::Address as _};
use soroban_token_template::{TokenContract, TokenContractClient};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_env_and_client(
    quorum: i128,
    execution_delay: u32,
) -> (Env, DaoContractClient, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_addr = env.register_contract(None, TokenContract);
    TokenContractClient::new(&env, &token_addr).initialize(
        &admin,
        &String::from_str(&env, "GOV"),
        &String::from_str(&env, "GOV"),
        &0u32,
        &None,
    );

    let dao_addr = env.register_contract(None, DaoContract);
    let client = DaoContractClient::new(&env, &dao_addr);
    client.initialize(&admin, &token_addr, &100u32, &quorum, &0u32, &execution_delay);

    (env, client, admin, token_addr)
}

fn mint(env: &Env, token: &Address, to: &Address, amount: i128) {
    TokenContractClient::new(env, token).mint(to, &amount);
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    /// Property: yes_votes + no_votes never exceed total_supply_at_creation.
    ///
    /// Because each voter's weight is capped at the snapshot supply and the
    /// same voter cannot vote twice, the aggregate vote count can never
    /// exceed the supply captured at proposal creation.
    #[test]
    fn prop_votes_never_exceed_snapshot_supply(
        num_voters in 1usize..=5usize,
        voter_amounts in proptest::collection::vec(1i128..=200i128, 1..=5),
    ) {
        let (env, client, admin, token) = make_env_and_client(1, 0);

        // Mint to proposer so they can create a proposal.
        let proposer = Address::generate(&env);
        mint(&env, &token, &proposer, 10);

        let id = client.create_proposal(
            &proposer,
            &String::from_str(&env, "Test"),
            &String::from_str(&env, "Invariant check"),
        ).unwrap();

        let snapshot = client.get_proposal(&id).total_supply_at_creation;

        for i in 0..num_voters.min(voter_amounts.len()) {
            let voter = Address::generate(&env);
            mint(&env, &token, &voter, voter_amounts[i]);
            let _ = client.try_vote(&voter, &id, &(i % 2 == 0));
        }

        let proposal = client.get_proposal(&id);
        let total_votes = proposal.yes_votes + proposal.no_votes;

        prop_assert!(
            total_votes <= snapshot,
            "total_votes ({}) exceeded snapshot supply ({})",
            total_votes,
            snapshot
        );
        let _ = admin; // suppress unused warning
    }

    /// Property: A proposal that never met quorum can never reach Executed state.
    ///
    /// With quorum = i128::MAX - 1 no realistic vote total can satisfy it, so
    /// queue_proposal must always return QuorumNotMet and the proposal stays Active.
    #[test]
    fn prop_execution_requires_quorum(
        voter_amount in 1i128..=1000i128,
    ) {
        // Set quorum far beyond any realistic vote total.
        let quorum = 1_000_000i128;
        let (env, client, _admin, token) = make_env_and_client(quorum, 0);

        let proposer = Address::generate(&env);
        mint(&env, &token, &proposer, voter_amount);

        let id = client.create_proposal(
            &proposer,
            &String::from_str(&env, "Q"),
            &String::from_str(&env, "test"),
        ).unwrap();

        let voter = Address::generate(&env);
        mint(&env, &token, &voter, voter_amount);
        let _ = client.try_vote(&voter, &id, &true);

        // Advance past deadline.
        let deadline = client.get_proposal(&id).deadline;
        env.ledger().with_mut(|l| l.sequence_number = deadline + 1);

        // Attempting to queue should fail because voter_amount < quorum (1_000_000).
        let queue_result = client.try_queue_proposal(&id);

        // Proposal must remain Active (not Queued/Executed).
        let state = client.get_proposal(&id).state;
        prop_assert!(
            state == ProposalState::Active,
            "Proposal should still be Active after quorum failure, got {:?}",
            state
        );
        prop_assert!(queue_result.is_err(), "queue_proposal should have failed");
    }
}
