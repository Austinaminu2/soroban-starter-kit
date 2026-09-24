# Testing Strategy

This document tracks what kind of test coverage exists for each contract template, so gaps are visible at a glance rather than discovered while debugging a regression.

## Test Types

| Type | What it checks | Where it lives |
|------|-----------------|-----------------|
| **Unit** | Individual entry points, error paths, auth checks | `contracts/<name>/src/test.rs` (or a `#[cfg(test)]` module in `lib.rs`) |
| **Property** | Invariants that must hold across randomized inputs (e.g. "supply is conserved", "no double-spend") | `contracts/<name>/src/prop_test.rs`, run via `proptest` |
| **Fuzz** | Crash/panic resistance against arbitrary byte input, via `cargo-fuzz` | `fuzz/fuzz_targets/*.rs` |
| **Integration** | Multiple deployed contracts interacting in one `Env` (e.g. token + escrow) | `tests/src/*.rs`, `tests/tests/*.rs` |

## Coverage Matrix

| Contract | Unit | Property | Fuzz | Integration |
|----------|:----:|:--------:|:----:|:------------:|
| `airdrop` | ✅ | ❌ | ❌ | ❌ |
| `auction` | ✅ | ❌ | ❌ | ✅ |
| `ballot` | ✅ | ❌ | ❌ | ❌ |
| `bonding-curve` | ✅ | ❌ | ❌ | ❌ |
| `crowdfund` | ✅ | ❌ | ❌ | ❌ |
| `dao` | ✅ | ❌ | ❌ | ❌ |
| `escrow` | ✅ | ✅ | ✅ | ✅ |
| `lottery` | ✅ | ❌ | ❌ | ✅ |
| `marketplace` | ✅ | ❌ | ❌ | ✅ |
| `multisig` | ✅ | ✅ | ❌ | ❌ |
| `nft` | ✅ | ✅ | ❌ | ✅ |
| `oracle` | ✅ | ❌ | ❌ | ❌ |
| `staking` | ✅ | ✅ | ❌ | ❌ |
| `subscription` | ✅ | ❌ | ❌ | ❌ |
| `swap` | ✅ | ❌ | ❌ | ❌ |
| `timelock` | ✅ | ❌ | ❌ | ❌ |
| `token` | ✅ | ✅ | ✅ | ✅ |
| `vesting` | ✅ | ✅ | ❌ | ❌ |
| `wrapped-token` | ✅ | ❌ | ❌ | ❌ |

_Generated against the contract list under `contracts/` and the test files present as of 2026-08-26; `contracts/common` is a shared library, not a deployable template, and is excluded. The Integration column reflects `tests/tests/integration.rs` as of this update (`auction`, `lottery`, `marketplace`, and `nft` gained coverage via #855/#856/#857/#863 since the matrix was last refreshed) — reconfirm against the file itself before trusting an older copy of this table._

## Gaps

- **No fuzz targets outside `token` and `escrow`.** `fuzz/fuzz_targets/` currently only has `token_fuzz.rs`, `token_mint_burn.rs`, and `escrow_initialize.rs`. Every other contract — including state-machine-heavy ones like `auction`, `lottery`, and `marketplace` — has no fuzz coverage. No tracking issue exists for this yet; file one against the `testing` area before picking it up so work isn't duplicated.
- **No property tests outside `escrow`, `multisig`, `nft`, `staking`, and `token`.** Contracts with numeric invariants worth property-testing (`bonding-curve` pricing curve, `auction` bid/refund accounting, `crowdfund` pledge/refund accounting, `lottery` payout accounting) currently rely on example-based unit tests only.
- **Integration tests still don't cover every contract that composes with a token in production** (`tests/tests/integration.rs`, tracking prior issues #221/#222/#855/#856/#857/#863, tracked further as [#971](https://github.com/Fidelis900/soroban-starter-kit/issues/971)). `escrow`, `token`, `auction`, `marketplace`, `lottery`, and `nft` now have real-token integration coverage; `airdrop`, `crowdfund`, `subscription`, `swap`, `vesting`, and `wrapped-token` still don't — each needs a happy-path test deploying a real token client rather than relying on unit tests against mocked storage alone.
- **`bonding-curve` and `wrapped-token`** have the thinnest unit suites (3 and 2 test functions respectively) relative to their entry-point count; see `docs/gas-costs.md` for their full entry-point lists.
- **Mutation testing (`cargo-mutants`) only examines `token` and `escrow`** (`mutants.toml`, tracked as [#972](https://github.com/Fidelis900/soroban-starter-kit/issues/972)). Mutation score measures whether a contract's *existing* unit tests actually kill injected bugs, which is a different signal from "a test.rs file exists" — several real fund-movement bugs (escrow's dispute vote-direction handling, staking's unbond-request overwrite, vesting's `admin_release`) have shipped past a green unit-test suite in contracts that weren't in mutation scope. `staking`, `vesting`, `escrow/src/dispute.rs`, `marketplace`, and `auction` are the priority additions.

When closing a gap, add a row update here in the same PR as the new tests so this matrix doesn't drift from reality.
