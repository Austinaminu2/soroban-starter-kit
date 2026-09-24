#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use crate::{BallotContract, BallotContractClient};
use proptest::prelude::*;
use soroban_sdk::{Address, Env, String, Vec, testutils::Address as _};

proptest! {
    /// Property: Total votes never exceed number of registered voters
    /// Closes #961 – ballot vote accounting invariant
    #[test]
    fn prop_votes_never_exceed_voters(
        num_voters in 1usize..=10usize,
        vote_attempts in 1usize..=20usize,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let ballot_addr = env.register_contract(None, BallotContract);
        let client = BallotContractClient::new(&env, &ballot_addr);

        let deadline = env.ledger().sequence() + 1000;

        let mut proposals = Vec::new(&env);
        proposals.push_back(String::from_str(&env, "Proposal 1"));
        proposals.push_back(String::from_str(&env, "Proposal 2"));

        client.initialize(&admin, &proposals, &deadline);

        // Register voters
        let mut voters = vec![];
        for _ in 0..num_voters {
            let voter = Address::generate(&env);
            let _ = client.try_register_voter(&voter);
            voters.push(voter);
        }

        // Attempt to vote multiple times
        let mut successful_votes = 0;
        for i in 0..vote_attempts {
            let voter_idx = i % voters.len();
            if client.try_vote(&voters[voter_idx], &0u32).is_ok() {
                successful_votes += 1;
            }
        }

        // Invariant: successful votes <= number of registered voters
        prop_assert!(successful_votes <= num_voters,
            "Votes exceeded voters: votes={}, voters={}", successful_votes, num_voters);
    }

    /// Property: Vote counts never go negative
    #[test]
    fn prop_vote_counts_never_negative(
        num_proposals in 1u32..=5u32,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let ballot_addr = env.register_contract(None, BallotContract);
        let client = BallotContractClient::new(&env, &ballot_addr);

        let deadline = env.ledger().sequence() + 1000;

        let mut proposals = Vec::new(&env);
        for i in 0..num_proposals {
            proposals.push_back(String::from_str(&env, &format!("Proposal {}", i)));
        }

        client.initialize(&admin, &proposals, &deadline);

        // Check all proposal vote counts
        for proposal_id in 0..num_proposals {
            let result = client.try_get_votes(&proposal_id);
            if let Ok(votes) = result {
                prop_assert!(votes >= 0, "Negative vote count for proposal {}: {}", proposal_id, votes);
            }
        }
    }
}
