# ADR 0008: Multi-sig Arbiter Design

- Status: Accepted
- Date: 2026-06-26

## Context

While the escrow contract initially used a single-arbiter model for dispute resolution, real-world use cases demand better resilience and reduced centralization risk. A single arbiter presents a single point of failure: key compromise, censorship, or deliberate misconduct can corrupt all dispute resolutions.

Multi-signature (multi-sig) dispute resolution improves fault tolerance and distributes trust, but introduces trade-offs in on-chain complexity, gas costs, and coordination overhead.

## Decision

We adopt an **on-chain vote accumulation pattern** for multi-sig arbiter resolution in escrow contracts. Multiple independent arbiters vote on-chain; once a threshold of affirmative votes is reached, the transaction executes atomically.

## Design Rationale

### Vote Accumulation Pattern

Each arbiter in an M-of-N group can independently vote to release or refund escrow funds:

1. Arbiters call `resolve_dispute(caller, release_to_seller_flag)` — either `true` (Release) or `false` (Refund)
2. Each vote is recorded on-chain in persistent storage separately by direction:
    - `ArbiterVotesRelease` → `Vec<Address>` for votes to release to seller
    - `ArbiterVotesRefund` → `Vec<Address>` for votes to refund to buyer
3. An arbiter who has already voted for one direction cannot vote for the opposite direction
4. When the vote count **for a single direction** reaches the threshold `required_signatures`, the transaction executes automatically in that direction
5. The resolution is atomic: funds transfer and state updates happen in a single transaction

### Benefits of on-chain accumulation

- **Transparency**: All votes are visible on-chain; no off-chain coordination needed
- **Atomic execution**: Once threshold is met, the outcome is guaranteed; no partial failures or race conditions
- **Auditability**: Vote history is permanently recorded; disputes can be investigated
- **Fault tolerance**: N-of-M means the system tolerates up to (M - N) arbiter failures or unavailability
- **No custom signing ceremonies**: Standard Stellar account signatures suffice; no threshold cryptography or MPC infrastructure required

### Gas efficiency via vote accumulation

- Instead of collecting M signatures off-chain and submitting a single multi-sig transaction, each arbiter submits an individual vote
- Each vote is a small transaction (just an address and resolution type); total on-chain footprint scales with M
- Avoids the need for a heavyweight multi-sig contract; escrow stores votes inline

## Consequences

### Short-term

- **Implementation**: Add `Arbiters` (Vec<Address>), `RequiredSignatures` (u32), and `ArbiterVotes` (Vec<Address>) fields to escrow storage
- **New entry point**: `vote_resolve(escrow_id, resolution_type, arbiter_address)` replaces single-arbiter `resolve()`
- **Backwards compatibility**: Single-arbiter escrows (legacy) continue to work; new contracts can opt into multi-sig at initialization
- **Gas cost**: Each vote transaction is small (~1000–2000 stroops per signature inclusion), but M votes are required total

### Long-term

- **Standardization**: Multi-sig arbiter pattern becomes a reusable template for other two-party contracts needing decentralized dispute resolution
- **Governance**: If a threshold of arbiters becomes unavailable, DAO governance can vote to replace arbiters or lower the threshold
- **Audit trail**: Vote records provide evidence for governance and compliance audits

## Alternatives Considered

| Alternative                        | Pros                                                | Cons                                                                                                  |
| ---------------------------------- | --------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| **Single arbiter (status quo)**    | Simple, low gas, fast                               | Single point of failure; centralization risk; key compromise is catastrophic                          |
| **Off-chain multi-sig collection** | Simpler for arbiters; fewer transactions            | Requires off-chain coordination; no transparency; no atomic guarantees; arbiters must trust submitter |
| **Threshold cryptography (MPC)**   | Strong security; single transaction                 | Complex to operate; specialized infrastructure; longer setup; not Stellar-native                      |
| **Timelock with escalation**       | Avoids arbiters; deterministic                      | Slow; unfair to sellers; doesn't resolve disputes                                                     |
| **On-chain DAO vote (this ADR)**   | Transparent; atomic; fault-tolerant; Stellar-native | More gas; higher latency; arbiters must stay online; votes are immutable                              |

## Migration and Upgrade Path

### For existing single-arbiter escrows

1. Escrow initialization includes an optional `arbiters` field:
    - If `arbiters` is empty/None → use legacy single-arbiter flow
    - If `arbiters` is non-empty → use multi-sig vote accumulation

2. No forced migration. Existing escrows remain on the single-arbiter path until manually upgraded or re-created.

### For new multi-sig escrows

1. Initialize escrow with `arbiters` = [arbiter_1, arbiter_2, ..., arbiter_M] and `required_signatures` = N
2. On dispute, arbiters independently call `resolve_dispute(caller, release_to_seller_flag)` with their chosen resolution
3. Votes are tracked separately by direction (release vs refund)
4. Once N votes accumulate **for the same direction**, funds release/refund automatically in that direction
5. An arbiter cannot change their vote once cast

### Governance upgrade scenario

If arbiters become unavailable:

1. DAO votes to approve a new arbiter set and threshold (if governance exists)
2. Admin calls `update_arbiters()` with new addresses and threshold
3. Pending disputes are resolved under the old arbiters; new disputes use new arbiters

## Implementation Notes

### Vote storage

```rust
#[contracttype]
pub enum DataKey {
    Arbiters,              // Vec<Address>
    RequiredSignatures,    // u32
    ArbiterVotesRelease,   // Vec<Address> — accumulates votes to release to seller
    ArbiterVotesRefund,    // Vec<Address> — accumulates votes to refund to buyer
}
```

### Vote event

```rust
pub fn voted(env: &Env, arbiter: &Address, escrow_id: &u64, resolution: &Symbol, vote_count: u32) {
    env.events().publish(
        (Symbol::new(env, "voted"), arbiter.clone()),
        (escrow_id, resolution.clone(), vote_count),
    );
}
```

### Threshold check

```rust
// Check votes for the direction the arbiter is voting for
let votes_key = if release_to_seller_flag {
    DataKey::ArbiterVotesRelease
} else {
    DataKey::ArbiterVotesRefund
};

let votes = env.storage()
    .instance()
    .get::<_, Vec<Address>>(&votes_key)
    .unwrap_or_else(|| Vec::new(&env));

// Reject if arbiter already voted for opposite direction
let opposite_key = if release_to_seller_flag {
    DataKey::ArbiterVotesRefund
} else {
    DataKey::ArbiterVotesRelease
};

let opposite_votes = env.storage()
    .instance()
    .get::<_, Vec<Address>>(&opposite_key)
    .unwrap_or_else(|| Vec::new(&env));

if opposite_votes.iter().any(|v| v == caller) {
    return Err(EscrowError::NotAuthorized);
}

// Add vote and check threshold
votes.push_back(caller.clone());
env.storage().instance().set(&votes_key, &votes);

if votes.len() >= required_signatures {
    // Execute resolution atomically in the direction with threshold votes
}
```

## Known Limitations and Future Work

1. **No vote revocation**: Once an arbiter votes for a direction, they cannot change to the opposite direction. This prevents flip-flopping and ensures vote integrity. If an arbiter made an error, they should coordinate off-chain with other arbiters.
2. **Conflicting votes deadlock**: If arbiters are split between release and refund and neither direction reaches threshold, the system deadlocks. Recommend threshold < M/2 or add a tie-breaking arbiter or timeout mechanism.
3. **No time-based expiry**: Votes accumulate indefinitely. Very long-lived disputes may need a "vote reset" mechanism after deadline + N ledgers.
4. **Arbiter rotation**: Changing the arbiter set mid-resolution may invalidate in-flight votes. Recommend version stamps or vote expiry tied to arbiter set changes.

## References

- [ADR 0006: Escrow Arbiter Model](0006-escrow-arbiter-model.md) — single-arbiter rationale and risks
- [ADR 0003: Admin Model](0003-admin-model.md) — admin role and authorization patterns
- [ADR 0001: Storage Tier Choices](0001-storage-tier-choices.md) — instance vs. persistent storage strategy
- [Escrow Contract: Multi-sig Arbiter Implementation](../contracts/escrow/README.md)
