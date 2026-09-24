# Integer Overflow Audit Checklist

This document tracks arithmetic operations across the 13 newer contracts that use `i128`/`u32` types, identifying which are verified-safe and which may need checked arithmetic.

## Status Legend

| Status | Meaning |
|--------|---------|
| ✅ Safe | Arithmetic is bounded by design or uses `checked_*` methods |
| ⚠️ Needs Review | Arithmetic could overflow with extreme inputs |
| 🔧 Fixed | Overflow has been addressed with checked arithmetic |

---

## bonding-curve

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| `reserve * PRICE_SCALE` | `calculate_price` | i128 | ✅ Safe | Bounded by `checked_add` in callers |
| `supply + 1` | `calculate_price` | i128 | ✅ Safe | Checked for zero |
| `supply.checked_add(amount)` | `buy_cost` | i128 | ✅ Safe | Uses `checked_add` |
| `amount * avg_price / PRICE_SCALE` | `buy_cost` | i128 | ⚠️ Needs Review | No overflow check on multiplication |
| `supply - amount` | `sell_proceeds` | i128 | ✅ Safe | `amount > supply` validated |
| `amount * avg_price / PRICE_SCALE` | `sell_proceeds` | i128 | ⚠️ Needs Review | No overflow check on multiplication |
| `supply + amount` | `buy` | i128 | ⚠️ Needs Review | Could overflow with large amounts |
| `reserve + cost` | `buy` | i128 | ⚠️ Needs Review | Could overflow with large reserves |

---

## lottery

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| `total.checked_add(split)` | `initialize` | u32 | ✅ Safe | Uses `checked_add` |
| `ticket_price * participants.len() as i128` | `draw` | i128 | ⚠️ Needs Review | No overflow check |
| `total_prize * split as i128 / BPS_DENOMINATOR as i128` | `draw` | i128 | ⚠️ Needs Review | No overflow check |
| `ticket_price * ticket_count` | `claim_refund` | i128 | ⚠️ Needs Review | No overflow check |
| `ticket_price * count as i128` | `claim_refund` | i128 | ⚠️ Needs Review | No overflow check |

---

## marketplace

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| `price * royalty_bps as i128 / 10_000` | `buy` | i128 | ✅ Safe | BPS validated ≤ 10_000 |
| `price - royalty` | `buy` | i128 | ✅ Safe | `royalty <= price` by BPS bound |
| `amount * royalty_bps as i128 / 10_000` | `accept_offer` | i128 | ✅ Safe | BPS validated ≤ 10_000 |
| `amount - royalty` | `accept_offer` | i128 | ✅ Safe | `royalty <= amount` by BPS bound |
| `id + 1` | `list_impl` | u64 | ⚠️ Needs Review | Could overflow with ~2^64 listings |

---

## crowdfund

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| `total_pledged + amount` | Various | i128 | ⚠️ Needs Review | No overflow check |

---

## staking

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| Various reward calculations | `claim_rewards` | i128 | ⚠️ Needs Review | Review precision arithmetic |

---

## airdrop

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| None identified | - | - | - | No arithmetic operations |

---

## auction

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| `highest_bid + min_increment` | `bid` | i128 | ⚠️ Needs Review | Could overflow |

---

## ballot

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| None identified | - | - | - | No arithmetic operations |

---

## dao

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| None identified | - | - | - | No arithmetic operations |

---

## escrow

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| None identified | - | - | - | No arithmetic operations |

---

## multisig

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| None identified | - | - | - | No arithmetic operations |

---

## nft

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| None identified | - | - | - | No arithmetic operations |

---

## oracle

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| None identified | - | - | - | No arithmetic operations |

---

## subscription

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| None identified | - | - | - | No arithmetic operations |

---

## timelock

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| None identified | - | - | - | No arithmetic operations |

---

## vesting

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| None identified | - | - | - | No arithmetic operations |

---

## wrapped-token

| Operation | Location | Type | Status | Notes |
|-----------|----------|------|--------|-------|
| None identified | - | - | - | No arithmetic operations |

---

## Follow-up Issues

The following arithmetic sites have been flagged for review:

1. **bonding-curve**: Multiplication overflow in `buy_cost` and `sell_proceeds`
2. **lottery**: Multiplication overflow in prize distribution
3. **marketplace**: `id + 1` overflow (extremely unlikely but possible)
4. **crowdfund**: `total_pledged + amount` overflow
5. **auction**: `highest_bid + min_increment` overflow
6. **staking**: Reward precision arithmetic

These should be filed as follow-up issues if not already tracked.
