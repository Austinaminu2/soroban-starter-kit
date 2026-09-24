#![no_std]
#![deny(missing_docs)]
//! Price oracle consumer contract template.
//!
//! An admin pushes signed price updates and consumers read them through
//! `get_price`, which rejects prices older than a configurable staleness
//! threshold.

#[cfg(test)]
extern crate std;

use soroban_sdk::{Address, Env, Vec, contract, contractimpl};

mod errors;
mod events;
mod storage;

pub use errors::OracleError;
pub use storage::{DataKey, PriceData, PriceObservation, PublisherSubmission};

use soroban_common::{LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD};

/// Maximum number of price observations retained for `get_twap`. Older
/// observations are dropped as new ones are recorded.
pub const MAX_HISTORY: u32 = 30;

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

fn get_required<T: soroban_sdk::TryFromVal<soroban_sdk::Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
) -> Result<T, OracleError> {
    env.storage()
        .instance()
        .get(key)
        .ok_or(OracleError::NotInitialized)
}

/// Compute the median of `values`, sorting them in place.
///
/// Uses insertion sort: authorized publisher sets are expected to stay small,
/// so this stays cheap while avoiding a dependency on the standard library.
#[allow(clippy::arithmetic_side_effects, clippy::integer_division)]
fn median(values: &mut Vec<i128>) -> i128 {
    let len = values.len();
    for i in 1..len {
        let key = values.get_unchecked(i);
        let mut j = i;
        while j > 0 && values.get_unchecked(j - 1) > key {
            let prev = values.get_unchecked(j - 1);
            values.set(j, prev);
            j -= 1;
        }
        values.set(j, key);
    }

    let mid = len / 2;
    #[allow(clippy::manual_is_multiple_of)] // is_multiple_of postdates the pinned 1.85.0 toolchain
    if len % 2 == 0 {
        values
            .get_unchecked(mid - 1)
            .saturating_add(values.get_unchecked(mid))
            / 2
    } else {
        values.get_unchecked(mid)
    }
}

/// Append a new price observation to the ring buffer, dropping the oldest
/// entry once [`MAX_HISTORY`] is exceeded.
fn push_history(env: &Env, price: i128, timestamp: u64) {
    let mut history: Vec<PriceObservation> = env
        .storage()
        .instance()
        .get(&DataKey::History)
        .unwrap_or(Vec::new(env));

    history.push_back(PriceObservation { price, timestamp });
    while history.len() > MAX_HISTORY {
        history.pop_front();
    }

    env.storage().instance().set(&DataKey::History, &history);
}

/// Compute the time-weighted average price over `observations`, which must be
/// non-empty and sorted oldest-first. Each observation's price is weighted by
/// how long it held (until the next observation, or `now` for the last one).
#[allow(clippy::arithmetic_side_effects, clippy::integer_division)]
fn twap(observations: &Vec<PriceObservation>, now: u64) -> i128 {
    let len = observations.len();
    if len == 1 {
        return observations.get_unchecked(0).price;
    }

    let mut weighted_sum: i128 = 0;
    let mut total_weight: i128 = 0;
    for i in 0..len {
        let obs = observations.get_unchecked(i);
        let next_timestamp = if i + 1 < len {
            observations.get_unchecked(i + 1).timestamp
        } else {
            now
        };
        let weight = i128::from(next_timestamp.saturating_sub(obs.timestamp));
        weighted_sum = weighted_sum.saturating_add(obs.price.saturating_mul(weight));
        total_weight = total_weight.saturating_add(weight);
    }

    if total_weight == 0 {
        return observations.get_unchecked(len - 1).price;
    }
    weighted_sum / total_weight
}

pub use contract::*;

// The `#[contract]` / `#[contractimpl]` macros generate an undocumented public
// client type. Confine the missing_docs allowance to this module (which also
// hosts the unit tests so the generated client's fields stay reachable) and
// re-export the public contract API above.
mod contract {
    #![allow(missing_docs)]
    use super::*;
    use storage::DataKey::*;

    /// Price oracle consumer contract.
    ///
    /// The admin pushes price updates; consumers call `get_price` which
    /// validates that the price is not stale before returning it.
    #[contract]
    pub struct OracleContract;

    #[contractimpl]
    impl OracleContract {
        /// Initialize with an admin and a staleness threshold (in ledgers).
        ///
        /// # Errors
        /// - [`OracleError::AlreadyInitialized`] if already set up.
        /// - [`OracleError::InvalidStalenessThreshold`] if threshold is zero.
        pub fn initialize(
            env: Env,
            admin: Address,
            staleness_threshold: u32,
        ) -> Result<(), OracleError> {
            if env.storage().instance().has(&Admin) {
                return Err(OracleError::AlreadyInitialized);
            }
            if staleness_threshold == 0 {
                return Err(OracleError::InvalidStalenessThreshold);
            }
            admin.require_auth();

            env.storage().instance().set(&Admin, &admin);
            env.storage()
                .instance()
                .set(&StalenessThreshold, &staleness_threshold);

            bump_instance(&env);
            events::initialized(&env, &admin, staleness_threshold);
            Ok(())
        }

        /// Push a new price. Admin only.
        ///
        /// # Errors
        /// - [`OracleError::NotInitialized`] if not yet set up.
        /// - [`OracleError::Unauthorized`] if caller is not the admin.
        pub fn update_price(env: Env, price: i128) -> Result<(), OracleError> {
            let admin: Address = get_required(&env, &Admin)?;
            admin.require_auth();

            let ledger = env.ledger().sequence();
            let timestamp = env.ledger().timestamp();
            env.storage().instance().set(&Price, &price);
            env.storage().instance().set(&UpdatedAt, &ledger);
            env.storage()
                .instance()
                .set(&UpdatedAtTimestamp, &timestamp);
            push_history(&env, price, timestamp);

            bump_instance(&env);
            events::price_updated(&env, &admin, price, ledger);
            Ok(())
        }

        /// Return the current price, rejecting it if it is stale.
        ///
        /// A price is stale when `current_ledger - updated_at > staleness_threshold`.
        ///
        /// # Errors
        /// - [`OracleError::NotInitialized`] if no price has been pushed yet.
        /// - [`OracleError::StalePrice`] if the price is older than the threshold.
        pub fn get_price(env: Env) -> Result<i128, OracleError> {
            let price: i128 = get_required(&env, &Price)?;
            let updated_at: u32 = get_required(&env, &UpdatedAt)?;
            let threshold: u32 = get_required(&env, &StalenessThreshold)?;

            let age = env.ledger().sequence().saturating_sub(updated_at);
            if age > threshold {
                return Err(OracleError::StalePrice);
            }

            bump_instance(&env);
            Ok(price)
        }

        /// Return the current price, rejecting it if it is older than `max_age` seconds.
        ///
        /// Unlike `get_price` (which enforces the fixed `staleness_threshold` configured
        /// at `initialize` time, measured in ledgers), this lets a caller specify its own
        /// tolerance in seconds, checked against `env.ledger().timestamp()`.
        ///
        /// # Errors
        /// - [`OracleError::NotInitialized`] if no price has been pushed yet.
        /// - [`OracleError::StalePrice`] if `env.ledger().timestamp() - last_updated > max_age`.
        pub fn get_price_checked(env: Env, max_age: u64) -> Result<i128, OracleError> {
            let price: i128 = get_required(&env, &Price)?;
            let updated_at_timestamp: u64 = get_required(&env, &UpdatedAtTimestamp)?;

            let age = env
                .ledger()
                .timestamp()
                .saturating_sub(updated_at_timestamp);
            if age > max_age {
                return Err(OracleError::StalePrice);
            }

            bump_instance(&env);
            Ok(price)
        }

        /// Return the raw price data (price, updated_at, admin, staleness_threshold).
        ///
        /// # Errors
        /// - [`OracleError::NotInitialized`] if not yet set up.
        pub fn get_price_data(env: Env) -> Result<PriceData, OracleError> {
            Ok(PriceData {
                price: get_required(&env, &Price)?,
                updated_at: get_required(&env, &UpdatedAt)?,
                admin: get_required(&env, &Admin)?,
                staleness_threshold: get_required(&env, &StalenessThreshold)?,
            })
        }

        /// Configure the set of addresses authorized to submit prices via `submit_price`.
        /// Admin only. Replaces any previously configured set.
        ///
        /// # Errors
        /// - [`OracleError::NotInitialized`] if not yet set up.
        /// - [`OracleError::Unauthorized`] if `admin` is not the configured admin.
        pub fn set_publishers(
            env: Env,
            admin: Address,
            publishers: Vec<Address>,
        ) -> Result<(), OracleError> {
            let stored_admin: Address = get_required(&env, &Admin)?;
            admin.require_auth();
            if admin != stored_admin {
                return Err(OracleError::Unauthorized);
            }

            env.storage().instance().set(&Publishers, &publishers);
            bump_instance(&env);
            Ok(())
        }

        /// Submit a price as an authorized publisher.
        ///
        /// # Errors
        /// - [`OracleError::Unauthorized`] if `publisher` is not in the set configured
        ///   via `set_publishers`.
        pub fn submit_price(env: Env, publisher: Address, price: i128) -> Result<(), OracleError> {
            publisher.require_auth();

            let publishers: Vec<Address> = env
                .storage()
                .instance()
                .get(&Publishers)
                .unwrap_or(Vec::new(&env));
            if !publishers.contains(&publisher) {
                return Err(OracleError::Unauthorized);
            }

            let timestamp = env.ledger().timestamp();
            let key = Submission(publisher.clone());
            env.storage()
                .persistent()
                .set(&key, &PublisherSubmission { price, timestamp });
            env.storage().persistent().extend_ttl(
                &key,
                LEDGER_LIFETIME_THRESHOLD,
                LEDGER_BUMP_AMOUNT,
            );
            push_history(&env, price, timestamp);
            bump_instance(&env);

            events::price_submitted(&env, &publisher, price, timestamp);
            Ok(())
        }

        /// Return the median of the latest submission from each authorized publisher,
        /// rather than trusting a single caller as `get_price` does.
        ///
        /// Submissions older than `max_staleness_seconds` are excluded from the median
        /// to prevent stale data from offline publishers from skewing the result.
        ///
        /// # Errors
        /// - [`OracleError::NotInitialized`] if `set_publishers` has never been called.
        /// - [`OracleError::NoPublisherData`] if no publisher has submitted a fresh price
        ///   (all submissions are older than `max_staleness_seconds` or no submissions exist).
        pub fn get_median_price(env: Env, max_staleness_seconds: u64) -> Result<i128, OracleError> {
            let publishers: Vec<Address> = get_required(&env, &Publishers)?;
            let now = env.ledger().timestamp();

            let mut prices: Vec<i128> = Vec::new(&env);
            for publisher in publishers.iter() {
                if let Some(submission) = env
                    .storage()
                    .persistent()
                    .get::<_, PublisherSubmission>(&Submission(publisher))
                {
                    let age = now.saturating_sub(submission.timestamp);
                    if age <= max_staleness_seconds {
                        prices.push_back(submission.price);
                    }
                }
            }

            if prices.is_empty() {
                return Err(OracleError::NoPublisherData);
            }

            Ok(median(&mut prices))
        }

        /// Compute the time-weighted average price over the last `window` seconds.
        ///
        /// Every `update_price` and `submit_price` call records an observation in a
        /// ring buffer of up to [`MAX_HISTORY`] entries; this averages the
        /// observations whose timestamp falls within `window` seconds of now,
        /// weighting each by how long it held before the next observation (or now,
        /// for the most recent one).
        ///
        /// # Errors
        /// - [`OracleError::InsufficientHistory`] if no observation falls within
        ///   `window` seconds of the current ledger timestamp.
        pub fn get_twap(env: Env, window: u64) -> Result<i128, OracleError> {
            let history: Vec<PriceObservation> = env
                .storage()
                .instance()
                .get(&History)
                .unwrap_or(Vec::new(&env));

            let now = env.ledger().timestamp();
            let cutoff = now.saturating_sub(window);

            let mut in_window: Vec<PriceObservation> = Vec::new(&env);
            for observation in history.iter() {
                if observation.timestamp >= cutoff {
                    in_window.push_back(observation);
                }
            }

            if in_window.is_empty() {
                return Err(OracleError::InsufficientHistory);
            }

            bump_instance(&env);
            Ok(twap(&in_window, now))
        }
    }
}

mod test;

#[cfg(test)]
mod prop_test;
