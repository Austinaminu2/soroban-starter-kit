#![no_std]
#![deny(missing_docs)]
//! Wrapped-token contract template.
//!
//! Wraps an underlying asset 1:1, minting wrapped tokens on deposit and
//! burning them on withdrawal, with an optional protocol fee in basis points
//! on each direction.

use soroban_sdk::{Address, Env, Symbol, Vec, contract, contractimpl, token, vec};

mod errors;
mod events;
mod storage;

pub use errors::WrappedTokenError;
pub use storage::DataKey;

use soroban_common::{
    BPS_DENOMINATOR, LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD, apply_bps_fee,
    extend_ttl_instance, extend_ttl_persistent,
};

fn bump(env: &Env) {
    extend_ttl_instance(env, LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

/// Returns `true` if `key` holds a `true` pause flag in instance storage.
#[cfg(feature = "pausable")]
fn flag(env: &Env, key: &DataKey) -> bool {
    env.storage().instance().get(key).unwrap_or(false)
}

/// Returns an error if wrapping is paused. The legacy `Paused` flag is honoured
/// so deployments paused before the asymmetric split stay paused after upgrade.
/// Only compiled when the `pausable` feature is enabled.
#[cfg(feature = "pausable")]
fn require_wrap_not_paused(env: &Env) -> Result<(), WrappedTokenError> {
    if flag(env, &DataKey::WrapPaused) || flag(env, &DataKey::Paused) {
        return Err(WrappedTokenError::Unauthorized);
    }
    Ok(())
}

/// Returns an error if unwrapping is paused. Only compiled when the `pausable`
/// feature is enabled.
#[cfg(feature = "pausable")]
fn require_unwrap_not_paused(env: &Env) -> Result<(), WrappedTokenError> {
    if flag(env, &DataKey::UnwrapPaused) || flag(env, &DataKey::Paused) {
        return Err(WrappedTokenError::Unauthorized);
    }
    Ok(())
}

/// Loads the admin address, or `NotInitialized` if the contract hasn't been set up.
/// Only compiled when the `pausable` feature is enabled.
#[cfg(feature = "pausable")]
fn require_admin(env: &Env) -> Result<Address, WrappedTokenError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(WrappedTokenError::NotInitialized)
}

/// Loads `(wrapped_token, underlying_token)`, or `NotInitialized`.
fn load_tokens(env: &Env) -> Result<(Address, Address), WrappedTokenError> {
    let wrapped_token: Address = env
        .storage()
        .instance()
        .get(&DataKey::WrappedToken)
        .ok_or(WrappedTokenError::NotInitialized)?;
    let underlying_token: Address = env
        .storage()
        .instance()
        .get(&DataKey::UnderlyingToken)
        .ok_or(WrappedTokenError::NotInitialized)?;
    Ok((wrapped_token, underlying_token))
}

/// Computes the fee on `amount` for the bps stored under `key` (0 if unset).
fn fee_for(env: &Env, key: &DataKey, amount: i128) -> Result<i128, WrappedTokenError> {
    let bps: u32 = env.storage().instance().get(key).unwrap_or(0);
    if bps == 0 {
        return Ok(0);
    }
    apply_bps_fee(amount, bps).ok_or(WrappedTokenError::Overflow)
}

fn load_fee_recipient(env: &Env) -> Result<Address, WrappedTokenError> {
    env.storage()
        .instance()
        .get(&DataKey::FeeRecipient)
        .ok_or(WrappedTokenError::InvalidFee)
}

/// Validates a batch and returns `(gross_total, fee_total, per-recipient net amounts)`.
fn split_batch(
    env: &Env,
    fee_key: &DataKey,
    recipients: &Vec<(Address, i128)>,
) -> Result<(i128, i128, Vec<i128>), WrappedTokenError> {
    if recipients.is_empty() {
        return Err(WrappedTokenError::EmptyBatch);
    }
    let mut gross_total = 0i128;
    let mut fee_total = 0i128;
    let mut nets = Vec::new(env);
    for (_, amount) in recipients.iter() {
        if amount <= 0 {
            return Err(WrappedTokenError::InvalidAmount);
        }
        let fee = fee_for(env, fee_key, amount)?;
        let net = amount
            .checked_sub(fee)
            .ok_or(WrappedTokenError::Overflow)?;
        if net <= 0 {
            return Err(WrappedTokenError::InvalidAmount);
        }
        gross_total = gross_total
            .checked_add(amount)
            .ok_or(WrappedTokenError::Overflow)?;
        fee_total = fee_total
            .checked_add(fee)
            .ok_or(WrappedTokenError::Overflow)?;
        nets.push_back(net);
    }
    Ok((gross_total, fee_total, nets))
}

/// Shared implementation of `wrap` and `wrap_batch`.
///
/// `user` pays `sum(amounts)` of the underlying asset: the fee portion goes to the
/// fee recipient and the net portion is moved into the reserve in one transfer each.
/// Each recipient is then minted its amount net of fee.
fn do_wrap(
    env: &Env,
    user: &Address,
    recipients: &Vec<(Address, i128)>,
) -> Result<(), WrappedTokenError> {
    if !env.storage().instance().has(&DataKey::Admin) {
        return Err(WrappedTokenError::NotInitialized);
    }
    #[cfg(feature = "pausable")]
    require_wrap_not_paused(env)?;
    let (gross_total, fee_total, nets) = split_batch(env, &DataKey::WrapFeeBps, recipients)?;
    user.require_auth();

    let (wrapped_token, underlying_token) = load_tokens(env)?;

    // The per-address cap bounds the cumulative gross amount `user` deposits,
    // summed across every recipient of a batch.
    if let Some(cap) = env
        .storage()
        .instance()
        .get::<DataKey, i128>(&DataKey::MaxWrapPerAddress)
    {
        let key = DataKey::WrappedByAddress(user.clone());
        let wrapped_by_user: i128 = env.storage().persistent().get(&key).unwrap_or(0i128);
        let new_user_total = wrapped_by_user
            .checked_add(gross_total)
            .ok_or(WrappedTokenError::Overflow)?;
        if new_user_total > cap {
            return Err(WrappedTokenError::MaxWrapExceeded);
        }
        env.storage().persistent().set(&key, &new_user_total);
        extend_ttl_persistent(env, &key, LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
    }

    let total: i128 = env
        .storage()
        .instance()
        .get(&DataKey::TotalWrapped)
        .unwrap_or(0i128);
    let net_total = gross_total
        .checked_sub(fee_total)
        .ok_or(WrappedTokenError::Overflow)?;
    let new_total = total
        .checked_add(net_total)
        .ok_or(WrappedTokenError::Overflow)?;

    let underlying = token::Client::new(env, &underlying_token);

    // Transfer the aggregated net amount from user to the reserve in a single call.
    underlying.transfer(user, &env.current_contract_address(), &net_total);

    if fee_total > 0 {
        let fee_recipient = load_fee_recipient(env)?;
        underlying.transfer(user, &fee_recipient, &fee_total);
        events::fee_collected(
            env,
            Symbol::new(env, "wrap"),
            user,
            &fee_recipient,
            fee_total,
        );
    }

    // Mint wrapped tokens to each recipient. `wrapped_token` must be a Stellar Asset
    // Contract (or other token exposing the admin-mint interface) with this contract
    // set as its admin.
    let minter = token::StellarAssetClient::new(env, &wrapped_token);
    let mut running_total = total;
    for ((recipient, _), net) in recipients.iter().zip(nets.iter()) {
        minter.mint(&recipient, &net);
        running_total = running_total
            .checked_add(net)
            .ok_or(WrappedTokenError::Overflow)?;
        events::wrapped(env, &recipient, net, running_total);
    }

    env.storage()
        .instance()
        .set(&DataKey::TotalWrapped, &new_total);

    bump(env);
    Ok(())
}

/// Shared implementation of `unwrap` and `unwrap_batch`.
///
/// Burns `sum(amounts)` wrapped tokens from `user` in a single call, then pays each
/// recipient its amount net of fee from the reserve and the aggregated fee to the
/// fee recipient.
fn do_unwrap(
    env: &Env,
    user: &Address,
    recipients: &Vec<(Address, i128)>,
) -> Result<(), WrappedTokenError> {
    if !env.storage().instance().has(&DataKey::Admin) {
        return Err(WrappedTokenError::NotInitialized);
    }
    #[cfg(feature = "pausable")]
    require_unwrap_not_paused(env)?;
    let (gross_total, fee_total, nets) =
        split_batch(env, &DataKey::UnwrapFeeBps, recipients)?;
    user.require_auth();

    let (wrapped_token, underlying_token) = load_tokens(env)?;

    let total: i128 = env
        .storage()
        .instance()
        .get(&DataKey::TotalWrapped)
        .unwrap_or(0i128);
    let new_total = total
        .checked_sub(gross_total)
        .ok_or(WrappedTokenError::Overflow)?;
    if new_total < 0 {
        return Err(WrappedTokenError::InsufficientReserve);
    }

    // Burn wrapped tokens from user
    token::Client::new(env, &wrapped_token).burn(user, &gross_total);

    let underlying = token::Client::new(env, &underlying_token);
    let this = env.current_contract_address();
    let mut running_total = total;
    for ((recipient, amount), net) in recipients.iter().zip(nets.iter()) {
        underlying.transfer(&this, &recipient, &net);
        running_total = running_total
            .checked_sub(amount)
            .ok_or(WrappedTokenError::Overflow)?;
        events::unwrapped(env, &recipient, net, running_total);
    }

    if fee_total > 0 {
        let fee_recipient = load_fee_recipient(env)?;
        underlying.transfer(&this, &fee_recipient, &fee_total);
        events::fee_collected(
            env,
            Symbol::new(env, "unwrap"),
            user,
            &fee_recipient,
            fee_total,
        );
    }

    env.storage()
        .instance()
        .set(&DataKey::TotalWrapped, &new_total);

    bump(env);
    Ok(())
}

/// Wrapped asset token contract.
///
/// Users deposit an underlying asset and receive an equivalent amount of wrapped tokens.
/// Users can burn wrapped tokens to retrieve the underlying asset.
///
/// Flow:
/// 1. Admin calls `initialize` — sets up the wrapped token and underlying asset addresses,
///    and optionally a per-address cap on cumulative wrapping.
/// 2. Users call `wrap` (or `wrap_batch`) to deposit underlying assets and mint wrapped
///    tokens (1:1 peg, net of any wrap fee).
/// 3. Users call `unwrap` (or `unwrap_batch`) to burn wrapped tokens and receive underlying
///    assets (1:1 peg, net of any unwrap fee).
///
/// `get_reserve_balance` lets off-chain monitoring assert the invariant that
/// `get_total_wrapped() <= get_reserve_balance()` always holds.
pub use contract::*;

// The `#[contract]` / `#[contractimpl]` macros generate an undocumented public
// client type. Confine the missing_docs allowance to this module and re-export
// the public contract API above, keeping the rest of the crate enforced.
mod contract {
    #![allow(missing_docs)]
    use super::*;

    #[contract]
    pub struct WrappedTokenContract;

    #[contractimpl]
    impl WrappedTokenContract {
        /// Initialize the wrapped token contract.
        ///
        /// `max_wrap_per_address` optionally bounds the cumulative amount any single
        /// address may ever wrap, to limit concentration risk. Pass `None` for no cap.
        ///
        /// `wrap_fee_bps` / `unwrap_fee_bps` optionally charge a protocol fee (in basis
        /// points, at most 10_000) on each wrap / unwrap, paid in the underlying asset to
        /// `fee_recipient`. `fee_recipient` is required when either fee is non-zero.
        ///
        /// # Errors
        /// - [`WrappedTokenError::AlreadyInitialized`] if called more than once.
        /// - [`WrappedTokenError::InvalidAmount`] if `max_wrap_per_address` is `Some(n)` with `n <= 0`.
        /// - [`WrappedTokenError::InvalidFee`] if a fee exceeds 10_000 bps, or a non-zero fee is
        ///   set without a `fee_recipient`.
        #[allow(clippy::too_many_arguments)]
        pub fn initialize(
            env: Env,
            admin: Address,
            wrapped_token: Address,
            underlying_token: Address,
            max_wrap_per_address: Option<i128>,
            wrap_fee_bps: Option<u32>,
            unwrap_fee_bps: Option<u32>,
            fee_recipient: Option<Address>,
        ) -> Result<(), WrappedTokenError> {
            if env.storage().instance().has(&DataKey::Admin) {
                return Err(WrappedTokenError::AlreadyInitialized);
            }
            admin.require_auth();

            let wrap_fee = wrap_fee_bps.unwrap_or(0);
            let unwrap_fee = unwrap_fee_bps.unwrap_or(0);
            let max_bps = u32::try_from(BPS_DENOMINATOR).unwrap_or(u32::MAX);
            if wrap_fee > max_bps || unwrap_fee > max_bps {
                return Err(WrappedTokenError::InvalidFee);
            }
            if (wrap_fee > 0 || unwrap_fee > 0) && fee_recipient.is_none() {
                return Err(WrappedTokenError::InvalidFee);
            }
            if wrap_fee > 0 {
                env.storage().instance().set(&DataKey::WrapFeeBps, &wrap_fee);
            }
            if unwrap_fee > 0 {
                env.storage()
                    .instance()
                    .set(&DataKey::UnwrapFeeBps, &unwrap_fee);
            }
            if let Some(recipient) = fee_recipient {
                env.storage()
                    .instance()
                    .set(&DataKey::FeeRecipient, &recipient);
            }

            env.storage().instance().set(&DataKey::Admin, &admin);
            env.storage()
                .instance()
                .set(&DataKey::WrappedToken, &wrapped_token);
            env.storage()
                .instance()
                .set(&DataKey::UnderlyingToken, &underlying_token);
            env.storage().instance().set(&DataKey::TotalWrapped, &0i128);

            if let Some(cap) = max_wrap_per_address {
                if cap <= 0 {
                    return Err(WrappedTokenError::InvalidAmount);
                }
                env.storage()
                    .instance()
                    .set(&DataKey::MaxWrapPerAddress, &cap);
            }

            bump(&env);
            events::initialized(&env, &admin, &wrapped_token);
            Ok(())
        }

        /// Wrap underlying assets by transferring them to the contract and minting wrapped tokens.
        ///
        /// 1:1 peg is maintained: `amount` underlying in, `amount - fee` wrapped tokens minted,
        /// where `fee = amount * wrap_fee_bps / 10_000` is sent to the fee recipient.
        ///
        /// # Errors
        /// - [`WrappedTokenError::NotInitialized`] if the contract has not been initialized.
        /// - [`WrappedTokenError::Unauthorized`] if wrapping is paused (requires the `pausable` feature).
        /// - [`WrappedTokenError::InvalidAmount`] if `amount` <= 0, or nothing is left after the fee.
        /// - [`WrappedTokenError::MaxWrapExceeded`] if `user`'s cumulative wrapped amount would exceed
        ///   the per-address cap set at `initialize`.
        /// - [`WrappedTokenError::Overflow`] if a supply calculation overflows `i128`.
        pub fn wrap(env: Env, user: Address, amount: i128) -> Result<(), WrappedTokenError> {
            do_wrap(&env, &user, &vec![&env, (user.clone(), amount)])
        }

        /// Wrap for many recipients at once. `user` deposits the sum of all amounts in a
        /// single underlying transfer (plus one fee transfer), and each recipient is minted
        /// its amount net of the wrap fee. The per-address cap is enforced against `user`'s
        /// cumulative deposits across the whole batch.
        ///
        /// # Errors
        /// Same as [`Self::wrap`], plus [`WrappedTokenError::EmptyBatch`] if `recipients` is empty.
        pub fn wrap_batch(
            env: Env,
            user: Address,
            recipients: Vec<(Address, i128)>,
        ) -> Result<(), WrappedTokenError> {
            do_wrap(&env, &user, &recipients)
        }

        /// Unwrap wrapped tokens by burning them and sending underlying assets back to the user.
        ///
        /// 1:1 peg is maintained: `amount` wrapped tokens burned, `amount - fee` underlying paid
        /// out, where `fee = amount * unwrap_fee_bps / 10_000` is sent to the fee recipient.
        ///
        /// # Errors
        /// - [`WrappedTokenError::NotInitialized`] if the contract has not been initialized.
        /// - [`WrappedTokenError::Unauthorized`] if unwrapping is paused (requires the `pausable` feature).
        /// - [`WrappedTokenError::InvalidAmount`] if `amount` <= 0, or nothing is left after the fee.
        /// - [`WrappedTokenError::InsufficientReserve`] if `amount` exceeds the total wrapped supply.
        pub fn unwrap(env: Env, user: Address, amount: i128) -> Result<(), WrappedTokenError> {
            do_unwrap(&env, &user, &vec![&env, (user.clone(), amount)])
        }

        /// Unwrap for many recipients at once. Burns the sum of all amounts from `user` in a
        /// single call, then pays each recipient its amount net of the unwrap fee.
        ///
        /// # Errors
        /// Same as [`Self::unwrap`], plus [`WrappedTokenError::EmptyBatch`] if `recipients` is empty.
        pub fn unwrap_batch(
            env: Env,
            user: Address,
            recipients: Vec<(Address, i128)>,
        ) -> Result<(), WrappedTokenError> {
            do_unwrap(&env, &user, &recipients)
        }

        /// Returns the total amount of wrapped tokens.
        pub fn get_total_wrapped(env: Env) -> i128 {
            env.storage()
                .instance()
                .get(&DataKey::TotalWrapped)
                .unwrap_or(0i128)
        }

        /// Returns the underlying asset's balance held by this contract, i.e. the actual
        /// reserve backing the wrapped supply.
        ///
        /// For monitoring: `get_total_wrapped() <= get_reserve_balance()` should always hold.
        ///
        /// # Errors
        /// - [`WrappedTokenError::NotInitialized`] if the contract has not been initialized.
        pub fn get_reserve_balance(env: Env) -> Result<i128, WrappedTokenError> {
            let underlying_token: Address = env
                .storage()
                .instance()
                .get(&DataKey::UnderlyingToken)
                .ok_or(WrappedTokenError::NotInitialized)?;
            Ok(
                token::Client::new(&env, &underlying_token)
                    .balance(&env.current_contract_address()),
            )
        }

        /// Returns the per-address wrap cap set at `initialize`, or `None` if uncapped.
        pub fn max_wrap_per_address(env: Env) -> Option<i128> {
            env.storage().instance().get(&DataKey::MaxWrapPerAddress)
        }

        /// Returns the cumulative amount `account` has wrapped so far (0 if never wrapped,
        /// or if no cap is configured and nothing was ever tracked).
        pub fn wrapped_by(env: Env, account: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&DataKey::WrappedByAddress(account))
                .unwrap_or(0i128)
        }

        /// Returns the wrap fee in basis points (0 if none).
        pub fn wrap_fee_bps(env: Env) -> u32 {
            env.storage()
                .instance()
                .get(&DataKey::WrapFeeBps)
                .unwrap_or(0)
        }

        /// Returns the unwrap fee in basis points (0 if none).
        pub fn unwrap_fee_bps(env: Env) -> u32 {
            env.storage()
                .instance()
                .get(&DataKey::UnwrapFeeBps)
                .unwrap_or(0)
        }

        /// Returns the fee recipient, or `None` if not configured.
        pub fn fee_recipient(env: Env) -> Option<Address> {
            env.storage().instance().get(&DataKey::FeeRecipient)
        }
    }

    /// Pause / unpause — only compiled when the `pausable` feature is enabled.
    #[cfg(feature = "pausable")]
    #[contractimpl]
    impl WrappedTokenContract {
        /// Pause both `wrap` and `unwrap`. Admin only.
        pub fn pause(env: Env) -> Result<(), WrappedTokenError> {
            let admin = require_admin(&env)?;
            admin.require_auth();
            env.storage().instance().set(&DataKey::WrapPaused, &true);
            env.storage().instance().set(&DataKey::UnwrapPaused, &true);
            bump(&env);
            events::paused(&env, &admin);
            Ok(())
        }

        /// Resume both `wrap` and `unwrap`. Admin only.
        pub fn unpause(env: Env) -> Result<(), WrappedTokenError> {
            let admin = require_admin(&env)?;
            admin.require_auth();
            env.storage().instance().set(&DataKey::WrapPaused, &false);
            env.storage().instance().set(&DataKey::UnwrapPaused, &false);
            env.storage().instance().remove(&DataKey::Paused);
            bump(&env);
            events::unpaused(&env, &admin);
            Ok(())
        }

        /// Pause or resume `wrap` / `wrap_batch` only, leaving unwrapping untouched so
        /// users can always exit during a risk event. Admin only.
        pub fn set_wrap_paused(env: Env, paused: bool) -> Result<(), WrappedTokenError> {
            let admin = require_admin(&env)?;
            admin.require_auth();
            env.storage().instance().set(&DataKey::WrapPaused, &paused);
            bump(&env);
            events::wrap_paused(&env, &admin, paused);
            Ok(())
        }

        /// Pause or resume `unwrap` / `unwrap_batch` only. Admin only.
        pub fn set_unwrap_paused(env: Env, paused: bool) -> Result<(), WrappedTokenError> {
            let admin = require_admin(&env)?;
            admin.require_auth();
            env.storage().instance().set(&DataKey::UnwrapPaused, &paused);
            bump(&env);
            events::unwrap_paused(&env, &admin, paused);
            Ok(())
        }

        /// Returns `true` if wrapping is paused.
        pub fn is_wrap_paused(env: Env) -> bool {
            flag(&env, &DataKey::WrapPaused) || flag(&env, &DataKey::Paused)
        }

        /// Returns `true` if unwrapping is paused.
        pub fn is_unwrap_paused(env: Env) -> bool {
            flag(&env, &DataKey::UnwrapPaused) || flag(&env, &DataKey::Paused)
        }
    }
}

#[cfg(test)]
mod test;

#[cfg(test)]
mod prop_test;
