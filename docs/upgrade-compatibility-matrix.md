# Upgrade Compatibility Matrix

Companion to the [Upgrade Guide](upgrade-guide.md): what storage each
contract actually persists, which contracts share a storage layout or data
structure with another contract (so a layout change in one must be applied
consistently to the other), and what kinds of code changes are
storage-breaking for a live deployment of each contract.

## How storage compatibility works here

- **`#[contracttype]` enums used as storage keys encode by variant *name*,
  not declaration order.** Every `storage.rs` in this repo documents this
  rule directly above its `DataKey` enum. Renaming or removing a variant
  changes the on-chain key and orphans existing data; adding a variant is
  safe as long as it's appended and no existing variant is renamed.
- **Struct shape matters.** For `#[contracttype]` structs actually written
  to storage (as opposed to response DTOs assembled at query time, see
  below), adding a field, removing a field, or changing a field's type is
  storage-breaking. Reordering `#[contracttype]` struct fields is also
  unsafe if the struct derives `Ord`/`PartialOrd`, since comparisons then
  depend on declaration order.
- **Not everything with a `#[contracttype]` on it is stored.** Most
  contracts expose a `get_info` / `get_*_data` query that assembles a
  response struct (`EscrowInfo`, `AuctionInfo`, `CrowdfundInfo`,
  `LotteryInfo`, `VestingInfo`, `PriceData`, `ListingEntry`, `ListingPage`,
  ...) from several individually-stored keys. These response DTOs can be
  changed freely between upgrades — they're never written to storage, only
  constructed on read. The matrix below lists only the types that are
  actually persisted.
- **Storage tier matters for TTL, not shape.** Moving a key between
  instance, persistent, and temporary storage doesn't change its XDR
  encoding, but changes its bump/eviction lifecycle — treat it as an
  operational change to test, even though it isn't "storage-breaking" in
  the layout sense.

## Contracts sharing a storage layout or data structure

| Pattern | Contracts | Why it matters |
|---|---|---|
| Admin address key | All 19 deployable contracts store a single admin `Address`. **`escrow`** is the only one that stores it under `soroban_common::AdminKey::Admin` (a shared type from the `common` crate) instead of a local `DataKey::Admin` variant — every other contract defines its own independent `DataKey::Admin`. A rename of `AdminKey::Admin` in `contracts/common` is storage-breaking for `escrow` specifically; it has no effect on the other 18, since their `Admin` variant lives in their own enum. |
| Upgrade timelock (`PendingUpgrade` + `Version`) | `escrow`, `token` | Both store the identical shape `PendingUpgrade: (BytesN<32>, u32)` = `(wasm_hash, ready_after_ledger)` plus a `Version: u32` counter, backing `propose_upgrade`/`execute_upgrade`. If this pattern is extended (e.g. to add a proposer address to the tuple), change both contracts together and update [`upgrade-guide.md`](upgrade-guide.md) — currently the only two contracts with on-chain upgrade timelocks. |
| Address-keyed persistent record | `airdrop` (`Claimed(Address)`), `auction` (`Pending(Address)`), `ballot` (`RegisteredVoter`/`Voter(Address)`), `crowdfund` (`Pledge(Address)`), `lottery` (`TicketCount(Address)`), `oracle` (`Submission(Address)`), `staking` (`Stake`/`RewardPerTokenPaid`/`Rewards`/`Compounding(Address)`), `subscription` (`Subscription(Address)`), `token` (`Balance`/`Frozen(Address)`) | Same *pattern* (an `Address` as part of the storage key) reused independently by each contract. Each contract's value shape is unrelated to the others' — this row is about template consistency, not a shared type. Changing one contract's per-address value shape doesn't affect any other contract. |
| Contract version counter | `escrow`, `multisig`, `staking`, `token`, `vesting` | All expose `contract_version()` backed by a `Version: u32` instance key. Only `escrow` and `token` pair it with `PendingUpgrade` (see above); in `multisig`/`staking`/`vesting` it is a plain counter with no upgrade-timelock storage attached. |

## Per-contract storage matrix

For each contract: the `DataKey` variants it defines (dynamic/persistent
keys marked), any `#[contracttype]` structs actually written to storage,
and change categories that would break storage for that contract
specifically, beyond the general rules above.

| Contract | Storage keys (`DataKey`) | Stored structs | Contract-specific storage-breaking risks |
|---|---|---|---|
| `airdrop` | `Admin`, `Token`, `MerkleRoot`, `ClaimDeadline`; dynamic: `Claimed(Address)` | — | None beyond the general rules. |
| `auction` | `Seller`, `Token`, `StartPrice`, `MinIncrement`, `Deadline`, `HighestBidder`, `HighestBid`, `Settled`, `ReservePrice`, `ExtensionWindow`, `Cancelled`; dynamic: `Pending(Address)` | — | None beyond the general rules. |
| `ballot` | `Admin`, `VotingActive`, `YesVotes`, `NoVotes`, `VotingStart`, `VotingEnd`, `TotalVotes`; dynamic: `RegisteredVoter(Address)`, `Voter(Address)` | — | None beyond the general rules. |
| `bonding-curve` | `Admin`, `Token`, `Reserve`, `Supply`, `Price` | — | None beyond the general rules. |
| `crowdfund` | `Creator`, `Token`, `Goal`, `Deadline`, `TotalPledged`, `Claimed`, `Tiers`, `MaxPledgePerAddress`, `DeadlineExtended`; dynamic: `Pledge(Address)` | `FundingTier` (stored inside the `Tiers: Vec<FundingTier>` list) | `FundingTier`'s field shape is storage-breaking since it's nested inside a stored `Vec`. `TierStatus` is a query-only DTO, not stored. |
| `dao` | `Admin`, `Token`, `VotingPeriod`, `Quorum`, `ProposalCount`, `Initialized` | `Proposal` (per-proposal, keyed by a separate `ProposalKey`), `VoteKey` | `Proposal`'s field shape and `ProposalState`'s variant names are storage-breaking. |
| `escrow` | `Buyer`, `Seller`, `Arbiter`, `TokenContract`, `Amount`, `Deadline`, `State`, `Paused`, `Version`, `PendingUpgrade`, `Arbiters`, `RequiredSignatures`, `ArbiterVotes`, `MetadataHash`, `DisputeTimeoutLedgers`, `DisputeRaisedAt`; admin via shared `soroban_common::AdminKey` (see table above) | `EscrowState` (stored under `State`) | `EscrowState` derives `Ord`/`PartialOrd` and lifecycle logic compares states with `<`/`>=`; reordering its variants changes comparison results even though the XDR key name doesn't change. Keep new states appended in lifecycle order, not just enum-declaration order. |
| `lottery` | `Admin`, `Token`, `TicketPrice`, `Participants`, `State`, `Winner`, `Winners`, `RevealDeadline`, `WinnerCount`, `PrizeSplits`, `MaxTicketsPerAddress`; dynamic: `TicketCount(Address)` | `Commit` (stored under `DataKey::Commit`) | None beyond the general rules. |
| `marketplace` | `Admin`, `PaymentToken`, `RoyaltyBps`, `RoyaltyRecipient`, `NextListingId`; dynamic: `Listing(u64)`, `Offer(u64, Address)` | `Listing` (stored per `Listing(u64)` key) | `Listing`'s field shape is storage-breaking; `ListingEntry`/`ListingPage` are pagination DTOs assembled at query time and not stored. |
| `multisig` | `Signers`, `Threshold`, `NextTransactionId`, `Paused`, `Version`; dynamic: `Transaction(u64)` | `Transaction` (stored per `Transaction(u64)` key) | `Transaction`'s field shape (including the `Vec<Val>` args it carries) is storage-breaking. |
| `nft` | `Admin`, `Name`, `Symbol`, `TotalSupply`, `MaxSupply`, `Initialized`; dynamic: per-token owner/approval/metadata keys (see `TokenKey`) | `TokenMetadata` | `TokenMetadata`'s field shape is storage-breaking. |
| `oracle` | `Admin`, `Price`, `UpdatedAt`, `StalenessThreshold`, `UpdatedAtTimestamp`, `Publishers`, `History`; dynamic: `Submission(Address)` | `PublisherSubmission` (stored per `Submission(Address)`), `PriceObservation` (stored inside the `History` ring buffer) | `PriceObservation`'s shape is storage-breaking since it's nested in the stored `History` vector. `PriceData` is a query-only DTO, not stored. |
| `staking` | `Admin`, `StakeToken`, `RewardToken`, `TotalStaked`, `TotalRewards`, `RewardPerTokenStored`, `Version`; dynamic: `Stake(Address)`, `RewardPerTokenPaid(Address)`, `Rewards(Address)`, `Compounding(Address)` | — | The `REWARD_SCALE` fixed-point scale used to interpret `RewardPerTokenStored`/`RewardPerTokenPaid` is a code constant, not stored — changing it changes how existing stored values are interpreted without any storage layout change, so treat it as storage-breaking in effect even though no key/type changes. |
| `subscription` | `Provider`, `Token`; dynamic: `Subscription(Address)` | `SubscriptionInfo` (stored per `Subscription(Address)`) | `SubscriptionInfo`'s field shape is storage-breaking. |
| `swap` | `SwapCount`, `Initialized`; dynamic (via `SwapKey`, see storage.rs) | `SwapInfo` | `SwapInfo`'s field shape and `SwapState`'s variant names are storage-breaking. |
| `timelock` | `Admin`, `Token`, `Beneficiary`, `ReleaseLedger`, `Amount`, `State` | `TimelockState` (stored under `State`) | `TimelockState`'s variant names are storage-breaking. |
| `token` | `Admin`, `PendingAdmin`, `TotalSupply`, `Paused`, `MaxSupply`, `PendingUpgrade`, `Version`, `TransferHook`; dynamic: `Balance(Address)`, `Allowance(AllowanceDataKey)`, `Metadata(MetadataKey)`, `Frozen(Address)`, `Snapshot(Address, u32)` | `AllowanceDataKey`/`AllowanceValue`, snapshot balances | `AllowanceDataKey`/`AllowanceValue`'s field shape is storage-breaking; it is also the type most likely to be read by external integrators (wallets, DEXs), so a shape change needs coordinated off-chain updates, not just an on-chain migration. Shares the `PendingUpgrade`/`Version` pattern with `escrow` (see table above). |
| `vesting` | `Beneficiary`, `Token`, `CliffLedger`, `EndLedger`, `Amount`, `Claimed`, `Revoked`, `Admin`, `AdminReleased`, `Version` | — | None beyond the general rules. |
| `wrapped-token` | `Admin`, `WrappedToken`, `UnderlyingToken`, `TotalWrapped`, `Paused`, `MaxWrapPerAddress`, `WrapPaused`, `UnwrapPaused`, `WrapFeeBps`, `UnwrapFeeBps`, `FeeRecipient`; dynamic: `WrappedByAddress(Address)` | — | The legacy `Paused` flag is kept and still honoured (it blocks both directions) so contracts paused before the wrap/unwrap pause split stay paused after upgrade; `unpause()` clears it. |

`contracts/common` has no storage of its own beyond the `AdminKey::Admin`
type it defines for `escrow` to reuse (see the shared-pattern table above);
it is not deployed independently.
