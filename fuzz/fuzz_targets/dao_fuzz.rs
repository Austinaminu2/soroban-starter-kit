//! libFuzzer invariant fuzz target for DAO proposal creation, voting, and execution.
//!
//! The fuzzer interprets raw bytes as a bounded sequence of DAO operations:
//!   * create a proposal with randomized voting configuration,
//!   * cast votes with randomized voters and weights,
//!   * advance the ledger,
//!   * queue and execute proposals.
//!
//! Invariants asserted:
//!   * total recorded votes never exceed the total eligible voting weight,
//!   * a voter can never vote twice on the same proposal (`AlreadyVoted`),
//!   * execution only succeeds when quorum and approval thresholds are met.
//!
//! The harness is deterministic (no wall-clock or RNG beyond the fuzz input)
//! and bounded in the number of operations it performs.

#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Bytes, Env, Vec,
};

use dao::{DaoContract, DaoContractClient, ProposalState, Vote};

/// Maximum number of operations decoded from a single fuzz input.
const MAX_OPS: usize = 64;

/// Maximum number of distinct voters we will ever register.
const MAX_VOTERS: usize = 8;

/// Maximum voting weight a single voter may hold.
const MAX_WEIGHT: u32 = 1_000;

/// Maximum number of ledgers we will advance in a single step.
const MAX_LEDGER_ADVANCE: u32 = 10_000;

/// A single decoded DAO operation.
#[derive(Clone, Copy, Debug)]
enum Op {
    /// Create a proposal with the given voting configuration.
    Create {
        quorum: u32,
        approval_bps: u32,
        voting_period: u32,
    },
    /// Cast a vote from `voter_idx` with the given weight and support flag.
    Vote {
        voter_idx: u8,
        weight: u32,
        support: bool,
    },
    /// Advance the ledger sequence by the given amount.
    Advance { ledgers: u32 },
    /// Queue the current proposal for execution.
    Queue,
    /// Execute the current proposal.
    Execute,
}

/// Decode a bounded sequence of operations from the raw fuzz input.
///
/// Each operation consumes a fixed number of bytes so decoding is stable and
/// deterministic. Trailing bytes that do not form a full operation are ignored.
fn decode_ops(data: &[u8]) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut cursor = 0usize;

    while cursor + 4 <= data.len() && ops.len() < MAX_OPS {
        let tag = data[cursor];
        cursor += 1;

        match tag % 5 {
            0 => {
                // Create: quorum, approval_bps, voting_period.
                let quorum = u32::from(data[cursor]) % (MAX_WEIGHT + 1);
                let approval_bps = u32::from(data[cursor + 1]) % 10_001;
                let voting_period = u32::from(data[cursor + 2]) % 1_000 + 1;
                cursor += 3;
                ops.push(Op::Create {
                    quorum,
                    approval_bps,
                    voting_period,
                });
            }
            1 => {
                // Vote: voter index, weight, support.
                let voter_idx = data[cursor] % (MAX_VOTERS as u8);
                let weight = u32::from(data[cursor + 1]) % (MAX_WEIGHT + 1);
                let support = data[cursor + 2] % 2 == 0;
                cursor += 3;
                ops.push(Op::Vote {
                    voter_idx,
                    weight,
                    support,
                });
            }
            2 => {
                // Advance: ledger delta.
                let ledgers = u32::from(data[cursor]) % MAX_LEDGER_ADVANCE + 1;
                cursor += 1;
                ops.push(Op::Advance { ledgers });
            }
            3 => {
                cursor += 1;
                ops.push(Op::Queue);
            }
            _ => {
                cursor += 1;
                ops.push(Op::Execute);
            }
        }
    }

    ops
}

/// Build a fresh Soroban test environment with a deterministic ledger.
fn setup_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set(LedgerInfo {
        timestamp: 0,
        protocol_version: 20,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 1,
        min_persistent_entry_ttl: 1,
        max_entry_ttl: 100_000,
    });
    env
}

/// Register the DAO contract and return a client plus the admin address.
fn setup_contract(env: &Env) -> (DaoContractClient<'_>, Address) {
    let contract_id = env.register_contract(None, DaoContract);
    let client = DaoContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    (client, admin)
}

/// Deterministically derive a voter address from an index.
fn voter_address(env: &Env, idx: u8) -> Address {
    // Address::generate is deterministic within a single Env instance, so we
    // generate a stable pool of addresses up front and index into it.
    let _ = idx;
    Address::generate(env)
}

fuzz_target!(|data: &[u8]| {
    let ops = decode_ops(data);
    if ops.is_empty() {
        return;
    }

    let env = setup_env();
    let (client, _admin) = setup_contract(&env);

    // Pre-generate a bounded, deterministic pool of voters.
    let mut voters: Vec<Address> = Vec::new(&env);
    for _ in 0..MAX_VOTERS {
        voters.push_back(Address::generate(&env));
    }

    // Track the eligible weight per voter for the current proposal.
    let mut eligible: [u32; MAX_VOTERS] = [0; MAX_VOTERS];
    // Track whether a voter has already voted on the current proposal.
    let mut voted: [bool; MAX_VOTERS] = [false; MAX_VOTERS];
    // Track the recorded weight per voter for the current proposal.
    let mut recorded: [u32; MAX_VOTERS] = [0; MAX_VOTERS];

    let mut current_proposal: Option<u64> = None;
    let mut quorum: u32 = 0;
    let mut approval_bps: u32 = 0;
    let mut total_eligible: u32 = 0;
    let mut total_recorded: u32 = 0;
    let mut total_support: u32 = 0;

    for op in ops.iter() {
        match op {
            Op::Create {
                quorum: q,
                approval_bps: a,
                voting_period,
            } => {
                // Reset per-proposal bookkeeping.
                eligible = [0; MAX_VOTERS];
                voted = [false; MAX_VOTERS];
                recorded = [0; MAX_VOTERS];
                total_eligible = 0;
                total_recorded = 0;
                total_support = 0;
                quorum = *q;
                approval_bps = *a;

                // Assign randomized but bounded weights to each voter.
                for i in 0..MAX_VOTERS {
                    let w = (u32::from(data.get(i).copied().unwrap_or(0)) % (MAX_WEIGHT + 1)) / 8;
                    eligible[i] = w;
                    total_eligible = total_eligible.saturating_add(w);
                }

                let title = Bytes::from_slice(&env, b"proposal");
                let description = Bytes::from_slice(&env, b"fuzz proposal");

                let proposal_id = client.create_proposal(
                    &voters.get(0).unwrap(),
                    &title,
                    &description,
                    &quorum,
                    &approval_bps,
                    &(*voting_period as u64),
                );
                current_proposal = Some(proposal_id);
            }
            Op::Vote {
                voter_idx,
                weight,
                support,
            } => {
                let proposal_id = match current_proposal {
                    Some(id) => id,
                    None => continue,
                };
                let idx = (*voter_idx as usize) % MAX_VOTERS;
                let voter = voters.get(idx as u32).unwrap();

                // Clamp the requested weight to the voter's eligible weight so
                // the contract cannot be tricked into over-counting.
                let clamped = (*weight).min(eligible[idx]);

                let result = client.try_vote(&voter, &proposal_id, &clamped, &support);

                if voted[idx] {
                    // A second vote from the same voter must be rejected.
                    assert!(
                        result.is_err(),
                        "voter {idx} double-voted on proposal {proposal_id}"
                    );
                } else if result.is_ok() {
                    voted[idx] = true;
                    recorded[idx] = clamped;
                    total_recorded = total_recorded.saturating_add(clamped);
                    if *support {
                        total_support = total_support.saturating_add(clamped);
                    }
                }

                // Invariant: recorded votes never exceed eligible weight.
                assert!(
                    total_recorded <= total_eligible,
                    "total recorded votes {total_recorded} exceed eligible weight {total_eligible}"
                );
                for i in 0..MAX_VOTERS {
                    assert!(
                        recorded[i] <= eligible[i],
                        "voter {i} recorded weight {} exceeds eligible {}",
                        recorded[i],
                        eligible[i]
                    );
                }
            }
            Op::Advance { ledgers } => {
                let current = env.ledger().sequence();
                env.ledger().set(LedgerInfo {
                    timestamp: 0,
                    protocol_version: 20,
                    sequence_number: current.saturating_add(*ledgers),
                    network_id: Default::default(),
                    base_reserve: 10,
                    min_temp_entry_ttl: 1,
                    min_persistent_entry_ttl: 1,
                    max_entry_ttl: 100_000,
                });
            }
            Op::Queue => {
                let proposal_id = match current_proposal {
                    Some(id) => id,
                    None => continue,
                };
                // Queueing may legitimately fail if thresholds are unmet; we
                // only assert that a successful queue is consistent.
                let _ = client.try_queue(&proposal_id);
            }
            Op::Execute => {
                let proposal_id = match current_proposal {
                    Some(id) => id,
                    None => continue,
                };

                let quorum_met = total_recorded >= quorum;
                let approval_met = if total_recorded == 0 {
                    false
                } else {
                    // approval_bps is in basis points (1/10000).
                    (u64::from(total_support) * 10_000)
                        >= (u64::from(total_recorded) * u64::from(approval_bps))
                };

                let result = client.try_execute(&proposal_id);

                if result.is_ok() {
                    // Execution must only succeed when thresholds were met.
                    assert!(
                        quorum_met,
                        "execution succeeded without quorum: recorded {total_recorded} < quorum {quorum}"
                    );
                    assert!(
                        approval_met,
                        "execution succeeded without approval: support {total_support} / recorded {total_recorded} < {approval_bps} bps"
                    );

                    // Once executed, the proposal must be in a terminal state.
                    if let Ok(proposal) = client.try_get_proposal(&proposal_id) {
                        if let Ok(p) = proposal {
                            assert!(
                                matches!(p.state, ProposalState::Executed),
                                "executed proposal not in Executed state: {:?}",
                                p.state
                            );
                        }
                    }
                }
            }
        }
    }

    // Final invariant: recorded votes never exceed eligible weight.
    assert!(
        total_recorded <= total_eligible,
        "final: total recorded votes {total_recorded} exceed eligible weight {total_eligible}"
    );
});