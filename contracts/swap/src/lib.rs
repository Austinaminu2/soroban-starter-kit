#![no_std]
#![deny(missing_docs)]
//! Atomic two-party token swap contract template.
//!
//! Party A escrows tokens when proposing a swap. Party B supplies the matching
//! tokens on acceptance; cancellation returns the escrow to Party A.

use soroban_sdk::{Address, Env, Vec, contract, contractimpl, token};

mod errors;
mod events;
mod storage;

pub use errors::SwapError;
pub use storage::{DataKey, SwapInfo, SwapState};
pub use storage::{DataKey, SwapInfo, SwapKey, SwapPage, SwapState};

use soroban_common::{
    LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD, apply_bps_fee, paginate,
};

/// Maximum number of swaps a single [`SwapContract::get_active_swaps`] call
/// may return, regardless of the requested `limit`.
pub const MAX_SWAPS_PAGE_SIZE: u32 = 50;

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

fn bump_swap(env: &Env, id: u32) {
    env.storage().persistent().extend_ttl(
        &DataKey::Swap(id),
        LEDGER_LIFETIME_THRESHOLD,
        LEDGER_BUMP_AMOUNT,
    );
}

fn get_instance<V: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
) -> Result<V, SwapError> {
    env.storage()
        .instance()
        .get(key)
        .ok_or(SwapError::NotInitialized)
}

/// Atomic two-party token swap.
/// Computes the protocol treasury fee and validates anti-fee-evasion constraints.
///
/// If `fee_bps > 0`, fee is computed via [`apply_bps_fee`]. If integer division truncates
/// the computed fee to 0 (which occurs on micro-transactions), a minimum fee policy is enforced:
/// the fee is set to `1` unit, provided `amount_b >= 1`.
/// The fee is strictly bounded such that `fee <= amount_b`.
pub fn calculate_and_validate_fee(amount_b: i128, fee_bps: u32) -> Result<i128, SwapError> {
    if fee_bps == 0 {
        return Ok(0);
    }
    let raw_fee = apply_bps_fee(amount_b, fee_bps).ok_or(SwapError::InvalidAmount)?;
    let fee = if raw_fee == 0 && amount_b > 0 {
        1i128
    } else {
        raw_fee
    };

    if fee > amount_b {
        return Err(SwapError::InvalidAmount);
    }

    Ok(fee)
}

/// Atomic two-party token swap contract.
pub use contract::*;

mod contract {
    #![allow(missing_docs)]
    use super::*;

    #[contract]
    pub struct SwapContract;

    #[contractimpl]
    impl SwapContract {
        /// Initialize the swap contract.
        pub fn initialize(
            env: Env,
            admin: Address,
            treasury: Address,
            fee_bps: u32,
        ) -> Result<(), SwapError> {
            if env.storage().instance().has(&DataKey::Initialized) {
            if env.storage().instance().has(&DataKey::Admin) {
                return Err(SwapError::AlreadyInitialized);
            }
            if fee_bps > 10_000 {
                return Err(SwapError::InvalidFee);
            }
            admin.require_auth();

            env.storage().instance().set(&DataKey::Admin, &admin);
            env.storage().instance().set(&DataKey::Treasury, &treasury);
            env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
            env.storage().instance().set(&DataKey::SwapCount, &0u32);
            env.storage().instance().set(&DataKey::Initialized, &true);

            extend_ttl_instance(&env);
            bump_instance(&env);
            Ok(())
        }

        /// Set the fee recipient. Only the administrator may call this.
        /// Update the treasury address. Only admin can call.
        ///
        /// # Errors
        /// - [`SwapError::NotInitialized`] if the contract is not initialized.
        /// - [`SwapError::NotAuthorized`] if caller is not the admin.
        pub fn set_treasury(env: Env, new_treasury: Address) -> Result<(), SwapError> {
            let admin: Address = get_instance(&env, &DataKey::Admin)?;
            admin.require_auth();
            env.storage()
                .instance()
                .set(&DataKey::Treasury, &new_treasury);
            bump_instance(&env);
            Ok(())
        }

        /// Set the fee in basis points. Only the administrator may call this.
        pub fn set_fee_bps(env: Env, new_fee_bps: u32) -> Result<(), SwapError> {
            let admin: Address = get_instance(&env, &DataKey::Admin)?;
            extend_ttl_instance(&env);

            Ok(())
        }

        /// Update the fee basis points. Only admin can call this.
        ///
        /// # Errors
        /// - [`SwapError::NotInitialized`] if the contract is not initialized.
        /// - [`SwapError::NotAuthorized`] if caller is not the admin.
        /// - [`SwapError::InvalidFee`] if `new_fee_bps` > 10000 (100%).
        pub fn set_fee_bps(env: Env, new_fee_bps: u32) -> Result<(), SwapError> {
            let admin: Address = get_required(&env, &DataKey::Admin)?;
            admin.require_auth();
            if new_fee_bps > 10_000 {
                return Err(SwapError::InvalidFee);
            }
            env.storage().instance().set(&DataKey::FeeBps, &new_fee_bps);
            bump_instance(&env);
            Ok(())
        }

        /// Set the administrator address. Only the current administrator may call this.
        /// Update the admin address. Only current admin can call this.
        ///
        /// # Errors
        /// - [`SwapError::NotInitialized`] if the contract is not initialized.
        /// - [`SwapError::NotAuthorized`] if caller is not the current admin.
        pub fn set_admin(env: Env, new_admin: Address) -> Result<(), SwapError> {
            let admin: Address = get_instance(&env, &DataKey::Admin)?;
            admin.require_auth();
            env.storage().instance().set(&DataKey::Admin, &new_admin);
            bump_instance(&env);
            Ok(())
        }

        /// Return the configured administrator.
            extend_ttl_instance(&env);
            bump_instance(&env);

            Ok(())
        }

        /// Get the current admin address.
        ///
        /// # Errors
        /// - [`SwapError::NotInitialized`] if the contract is not initialized.
        pub fn get_admin(env: Env) -> Result<Address, SwapError> {
            get_instance(&env, &DataKey::Admin)
        }

        /// Return the configured treasury.
        /// Get the current treasury address.
        ///
        /// # Errors
        /// - [`SwapError::NotInitialized`] if the contract is not initialized.
        pub fn get_treasury(env: Env) -> Result<Address, SwapError> {
            get_instance(&env, &DataKey::Treasury)
        }

        /// Return the configured fee in basis points.
        pub fn get_fee_bps(env: Env) -> Result<u32, SwapError> {
            get_instance(&env, &DataKey::FeeBps)
        }

        /// Return the number of swaps created so far.
        pub fn swap_count(env: Env) -> Result<u32, SwapError> {
            get_instance(&env, &DataKey::SwapCount)
        }

        /// Propose a swap and escrow Party A's asset in persistent storage.
        /// Get the current fee basis points.
        ///
        /// # Errors
        /// - [`SwapError::NotInitialized`]
        pub fn get_fee_bps(env: Env) -> Result<u32, SwapError> {
            get_required(&env, &DataKey::FeeBps)
        }

        /// Return the total number of swaps created.
        ///
        /// # Errors
        /// - [`SwapError::NotInitialized`]
        pub fn swap_count(env: Env) -> Result<u32, SwapError> {
            get_required(&env, &DataKey::SwapCount)
        }

        /// Propose a new swap. Party A proposes a swap of `amount_a` of `token_a`
        /// for `amount_b` of `token_b`. Returns the swap ID.
        ///
        /// The swap is valid until `expires_at` ledger.
        ///
        /// # Errors
        /// - [`SwapError::NotInitialized`]
        /// - [`SwapError::InvalidAmount`] if amounts <= 0.
        /// - [`SwapError::InvalidDeadline`] if `expires_at` <= current ledger.
        #[allow(clippy::too_many_arguments)]
        pub fn propose_swap(
            env: Env,
            party_a: Address,
            token_a: Address,
            amount_a: i128,
            token_b: Address,
            amount_b: i128,
            expires_at: u32,
            allowed_counterparty: Option<Address>,
            max_execution_delay: Option<u32>,
        ) -> Result<u32, SwapError> {
            let _admin: Address = get_instance(&env, &DataKey::Admin)?;
            party_a.require_auth();
            if amount_a <= 0 || amount_b <= 0 {
                return Err(SwapError::InvalidAmount);
            }
            if expires_at <= env.ledger().sequence() {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(SwapError::NotInitialized);
            }

            party_a.require_auth();

            if amount_a <= 0 || amount_b <= 0 {
                return Err(SwapError::InvalidAmount);
            }

            let current_ledger = env.ledger().sequence();
            if expires_at <= current_ledger {
                return Err(SwapError::InvalidDeadline);
            }
            let id: u32 = get_instance(&env, &DataKey::SwapCount)?;
            let next_id = id.checked_add(1).ok_or(SwapError::InvalidAmount)?;

            // Escrow token_a from party_a to this contract
            token::Client::new(&env, &token_a).transfer(
                &party_a,
                &env.current_contract_address(),
                &amount_a,
            );
            let swap = SwapInfo {
                id,

            let swap_id: u32 = env
                .storage()
                .instance()
                .get(&DataKey::SwapCount)
                .unwrap_or(0);
            let next_id = swap_id.checked_add(1).ok_or(SwapError::InvalidAmount)?;
            env.storage().instance().set(&DataKey::SwapCount, &next_id);

            let swap = SwapInfo {
                id: swap_id,
                party_a: party_a.clone(),
                token_a: token_a.clone(),
                amount_a,
                token_b: token_b.clone(),
                amount_b,
                expires_at,
                state: SwapState::Open,
            };
            env.storage().persistent().set(&DataKey::Swap(id), &swap);
            env.storage().instance().set(&DataKey::SwapCount, &next_id);
            bump_swap(&env, id);
            bump_instance(&env);
            events::swap_proposed(
                &env, &party_a, id, &token_a, amount_a, &token_b, amount_b, expires_at,
            );
            Ok(id)
        }

        /// Accept an open swap and atomically exchange both parties' assets.
        pub fn accept_swap(env: Env, swap_id: u32, party_b: Address) -> Result<u32, SwapError> {
            let treasury: Address = get_instance(&env, &DataKey::Treasury)?;
            let fee_bps: u32 = get_instance(&env, &DataKey::FeeBps)?;
                state: SwapState::Pending,
                allowed_counterparty: allowed_counterparty.clone(),
                max_execution_delay,
                created_at: current_ledger,
            };
            env.storage()
                .persistent()
                .set(&SwapKey::Swap(swap_id), &swap);

            extend_ttl_persistent(&env, &SwapKey::Swap(swap_id));
            bump_persistent(&env, &SwapKey::Swap(swap_id));
            bump_instance(&env);

            events::swap_proposed(
                &env,
                &party_a,
                swap_id,
                &token_a,
                amount_a,
                &token_b,
                amount_b,
                expires_at,
                &allowed_counterparty,
            );
            Ok(swap_id)
        }

        /// Accept a proposed swap. Party B accepts and the swap executes atomically.
        ///
        /// # Errors
        /// - [`SwapError::NotInitialized`]
        /// - [`SwapError::SwapNotFound`]
        /// - [`SwapError::InvalidState`] / [`SwapError::AlreadyCompleted`]
        /// - [`SwapError::DeadlineExpired`]
        /// - [`SwapError::NotAuthorized`] if restricted taker does not match
        /// - [`SwapError::ExecutionDelayExceeded`] if max delay window is exceeded
        pub fn accept_swap(
            env: Env,
            swap_id: u32,
            party_b: Address,
        ) -> Result<u32, SwapError> {
            party_b.require_auth();
            let mut swap: SwapInfo = env
                .storage()
                .persistent()
                .get(&DataKey::Swap(swap_id))
                .ok_or(SwapError::SwapNotFound)?;
            match swap.state {
                SwapState::Completed => return Err(SwapError::AlreadyCompleted),
                SwapState::Cancelled => return Err(SwapError::AlreadyCancelled),
                SwapState::Open => {}
            }
            if env.ledger().sequence() > swap.expires_at {
                return Err(SwapError::DeadlineExpired);

            if swap.state == SwapState::Accepted {
                return Err(SwapError::AlreadyCompleted);
            }
            if swap.state == SwapState::Cancelled {
                return Err(SwapError::AlreadyCancelled);
            }
            if swap.state != SwapState::Pending {
                return Err(SwapError::InvalidState);
            }

            let current_ledger = env.ledger().sequence();
            if current_ledger > swap.expires_at {
                return Err(SwapError::DeadlineExpired);
            }

            // Verify allowed counterparty restriction if configured
            if let Some(ref allowed) = swap.allowed_counterparty {
                if party_b != *allowed {
                    return Err(SwapError::NotAuthorized);
                }
            }

            // Verify maximum execution delay window check if configured
            if let Some(max_delay) = swap.max_execution_delay {
                let max_allowed_ledger = swap.created_at.saturating_add(max_delay);
                if current_ledger > max_allowed_ledger {
                    return Err(SwapError::ExecutionDelayExceeded);
                }
            }
            let fee = apply_bps_fee(swap.amount_b, fee_bps).unwrap_or(0);
            let party_a_amount = swap
                .amount_b
                .checked_sub(fee)
                .ok_or(SwapError::InvalidAmount)?;

            // Effects are recorded before interactions; Soroban transactions revert
            // all writes if a token transfer fails.
            swap.state = SwapState::Completed;
            env.storage()
                .persistent()
                .set(&DataKey::Swap(swap_id), &swap);
            bump_swap(&env, swap_id);
                .set(&SwapKey::Swap(swap_id), &swap);
            extend_ttl_persistent(&env, &SwapKey::Swap(swap_id));
            bump_persistent(&env, &SwapKey::Swap(swap_id));
            bump_instance(&env);

            // Calculate fee with anti-evasion minimum fee enforcement
            let fee = calculate_and_validate_fee(swap.amount_b, fee_bps)?;
            #[allow(clippy::arithmetic_side_effects)]
            let party_a_amount = swap.amount_b - fee;

            // Party B sends token_b to this contract
            token::Client::new(&env, &swap.token_b).transfer(
                &party_b,
                &env.current_contract_address(),
                &swap.amount_b,
            );

            // Contract forwards token_b minus fee to party A
            token::Client::new(&env, &swap.token_b).transfer(
                &env.current_contract_address(),
                &swap.party_a,
                &party_a_amount,
            );

            // Forward fee to treasury if fee > 0
            if fee > 0 {
                token::Client::new(&env, &swap.token_b).transfer(
                    &env.current_contract_address(),
                    &treasury,
                    &fee,
                );
            }

            // Contract forwards escrowed token_a to party B
            token::Client::new(&env, &swap.token_a).transfer(
                &env.current_contract_address(),
                &party_b,
                &swap.amount_a,
            );
            bump_instance(&env);
            events::swap_accepted(&env, &party_b, swap_id);
            Ok(swap_id)
        }

        /// Cancel an open swap and return Party A's escrowed asset.
        /// Cancel a pending swap. Party A can cancel before expiry; anyone can cancel after expiry.
        ///
        /// # Errors
        /// - [`SwapError::NotInitialized`]
        /// - [`SwapError::SwapNotFound`]
        /// - [`SwapError::AlreadyCompleted`] / [`SwapError::AlreadyCancelled`]
        /// - [`SwapError::NotAuthorized`]
        pub fn cancel_swap(env: Env, swap_id: u32) -> Result<(), SwapError> {
            if !env.storage().instance().has(&DataKey::Admin) {
                return Err(SwapError::NotInitialized);
            }

            let mut swap: SwapInfo = env
                .storage()
                .persistent()
                .get(&DataKey::Swap(swap_id))
                .ok_or(SwapError::SwapNotFound)?;
            match swap.state {
                SwapState::Completed => return Err(SwapError::AlreadyCompleted),
                SwapState::Cancelled => return Err(SwapError::AlreadyCancelled),
                SwapState::Open => {}
            }
            swap.party_a.require_auth();

            if swap.state == SwapState::Accepted {
                return Err(SwapError::AlreadyCompleted);
            }
            if swap.state == SwapState::Cancelled {
                return Err(SwapError::AlreadyCancelled);
            }
            if swap.state != SwapState::Pending {
                return Err(SwapError::InvalidState);
            }

            let current_ledger = env.ledger().sequence();
            let is_expired = current_ledger > swap.expires_at;

            if !is_expired {
                swap.party_a.require_auth();
            }

            swap.state = SwapState::Cancelled;
            env.storage()
                .persistent()
                .set(&DataKey::Swap(swap_id), &swap);
            bump_swap(&env, swap_id);
            token::Client::new(&env, &swap.token_a).transfer(
                &env.current_contract_address(),
                &swap.party_a,
                &swap.amount_a,
            );
            bump_instance(&env);

            // Refund escrowed token_a back to party A
            token::Client::new(&env, &swap.token_a).transfer(
                &env.current_contract_address(),
                &swap.party_a,
                &swap.amount_a,
            );

            events::swap_cancelled(&env, swap_id);
            Ok(())
        }

        /// Return a swap from persistent storage.
        /// Get swap details.
        ///
        /// # Errors
        /// - [`SwapError::SwapNotFound`]
        pub fn get_swap(env: Env, swap_id: u32) -> Result<SwapInfo, SwapError> {
            let swap: SwapInfo = env
                .storage()
                .persistent()
                .get(&DataKey::Swap(swap_id))
                .ok_or(SwapError::SwapNotFound)?;
            bump_swap(&env, swap_id);
            Ok(swap)
        }

        /// Enumerate active (pending and non-expired) swaps, paginated by swap ID.
        ///
        /// `cursor` is the swap ID to resume scanning from (pass `0` to start from the beginning).
        /// `limit` is the maximum number of active swaps to return and is capped at [`MAX_SWAPS_PAGE_SIZE`].
        ///
        /// Uses [`soroban_common::paginate`] to filter deterministic active listings.
        pub fn get_active_swaps(env: Env, cursor: u32, limit: u32) -> SwapPage {
            let total_count: u32 = env
                .storage()
                .instance()
                .get(&DataKey::SwapCount)
                .unwrap_or(0);

            let current_ledger = env.ledger().sequence();

            let page = paginate(
                &env,
                cursor,
                limit,
                MAX_SWAPS_PAGE_SIZE,
                |c| {
                    let next = c.saturating_add(1);
                    if next < total_count {
                        Some(next)
                    } else {
                        None
                    }
                },
                |c| {
                    if c >= total_count {
                        return None;
                    }
                    let swap: SwapInfo = env.storage().persistent().get(&SwapKey::Swap(c))?;
                    if swap.state == SwapState::Pending && current_ledger <= swap.expires_at {
                        Some(swap)
                    } else {
                        None
                    }
                },
            );

            SwapPage {
                swaps: page.items,
                next_cursor: page.next_cursor,
            }
        }
    }
}

#[cfg(test)]
mod prop_test;
mod test;
