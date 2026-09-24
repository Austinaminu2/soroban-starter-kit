#![no_std]
#![deny(missing_docs)]
//! Recurring subscription payment contract template.
//!
//! A provider charges a subscriber a fixed amount per interval by pulling from
//! a pre-approved token allowance until the subscriber cancels.

use soroban_sdk::{Address, Env, Symbol, contract, contractimpl, token};

mod errors;
mod events;
mod storage;

pub use errors::SubscriptionError;
pub use storage::{DataKey, Plan, SubscriptionInfo};

use soroban_common::{LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD};

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

fn bump_subscription(env: &Env, subscriber: &Address) {
    env.storage().persistent().extend_ttl(
        &DataKey::Subscription(subscriber.clone()),
        LEDGER_LIFETIME_THRESHOLD,
        LEDGER_BUMP_AMOUNT,
    );
}

/// Recurring payment subscription contract.
///
/// Lifecycle:
/// 1. Deployer calls `initialize` to set the provider address and payment token.
/// 2. Subscribers call `subscribe` to register a recurring payment plan.
///    The subscriber must grant a token allowance to this contract so the provider
///    can pull payments: `token.approve(subscriber, this_contract, amount * N, expiry)`.
/// 3. Provider calls `charge(subscriber)` once per interval to collect payment.
/// 4. Subscriber calls `cancel` to stop future charges.
pub use contract::*;

// The `#[contract]` / `#[contractimpl]` macros generate an undocumented public
// client type. Confine the missing_docs allowance to this module and re-export
// the public contract API above, keeping the rest of the crate enforced.
mod contract {
    #![allow(missing_docs)]
    use super::*;

    #[contract]
    pub struct SubscriptionContract;

    #[contractimpl]
    impl SubscriptionContract {
        /// Initialize the contract with a provider and payment token. Must be called exactly once.
        ///
        /// # Errors
        ///
        /// Returns [`SubscriptionError::AlreadyInitialized`] if already initialized.
        pub fn initialize(
            env: Env,
            provider: Address,
            token: Address,
        ) -> Result<(), SubscriptionError> {
            if env.storage().instance().has(&DataKey::Provider) {
                return Err(SubscriptionError::AlreadyInitialized);
            }

            provider.require_auth();

            // Validate that token implements the token interface.
            token::Client::new(&env, &token).decimals();

            env.storage().instance().set(&DataKey::Provider, &provider);
            env.storage().instance().set(&DataKey::Token, &token);
            bump_instance(&env);

            events::initialized(&env, &provider, &token);
            Ok(())
        }

        /// Register a new subscription plan. Only the provider (admin) can call this.
        ///
        /// # Errors
        ///
        /// Returns [`SubscriptionError::NotInitialized`] if the contract is not initialized.
        /// Returns [`SubscriptionError::NotAuthorized`] if the caller is not the provider.
        /// Returns [`SubscriptionError::InvalidAmount`] if `amount` <= 0.
        /// Returns [`SubscriptionError::InvalidInterval`] if `interval_ledgers` == 0.
        /// Returns [`SubscriptionError::PlanAlreadyExists`] if a plan with this ID already exists.
        pub fn register_plan(
            env: Env,
            plan_id: Symbol,
            amount: i128,
            interval_ledgers: u32,
        ) -> Result<(), SubscriptionError> {
            let provider: Address = env
                .storage()
                .instance()
                .get(&DataKey::Provider)
                .ok_or(SubscriptionError::NotInitialized)?;

            provider.require_auth();

            if amount <= 0 {
                return Err(SubscriptionError::InvalidAmount);
            }
            if interval_ledgers == 0 {
                return Err(SubscriptionError::InvalidInterval);
            }

            let key = DataKey::Plan(plan_id.clone());
            if env.storage().persistent().has(&key) {
                return Err(SubscriptionError::PlanAlreadyExists);
            }

            let plan = Plan {
                plan_id: plan_id.clone(),
                amount,
                interval_ledgers,
                active: true,
            };

            env.storage().persistent().set(&key, &plan);
            bump_instance(&env);

            events::plan_registered(&env, &plan_id, amount, interval_ledgers);
            Ok(())
        }

        /// Update an existing plan's status. Only the provider (admin) can call this.
        ///
        /// Deactivating a plan (`active = false`) only prevents **new subscriptions** from
        /// being created for this plan. Existing subscribers who are already subscribed to
        /// the plan will continue to be charged on their normal billing cycle. To stop
        /// billing for existing subscribers, they must call `cancel` individually.
        ///
        /// # Errors
        ///
        /// Returns [`SubscriptionError::NotInitialized`] if the contract is not initialized.
        /// Returns [`SubscriptionError::NotAuthorized`] if the caller is not the provider.
        /// Returns [`SubscriptionError::PlanNotFound`] if no plan exists with this ID.
        pub fn set_plan_active(
            env: Env,
            plan_id: Symbol,
            active: bool,
        ) -> Result<(), SubscriptionError> {
            let provider: Address = env
                .storage()
                .instance()
                .get(&DataKey::Provider)
                .ok_or(SubscriptionError::NotInitialized)?;

            provider.require_auth();

            let key = DataKey::Plan(plan_id.clone());
            let mut plan: Plan = env
                .storage()
                .persistent()
                .get(&key)
                .ok_or(SubscriptionError::PlanNotFound)?;

            plan.active = active;
            env.storage().persistent().set(&key, &plan);
            bump_instance(&env);

            events::plan_updated(&env, &plan_id, active);
            Ok(())
        }

        /// Register a recurring payment subscription by selecting an existing plan.
        ///
        /// The subscriber must pre-approve this contract as a spender on the payment token
        /// before the provider can call `charge`. Concretely, the subscriber should call
        /// `token.approve(subscriber, subscription_contract, amount * periods, expiry_ledger)`
        /// with enough allowance to cover the desired number of billing periods.
        ///
        /// # Errors
        ///
        /// Returns [`SubscriptionError::NotInitialized`] if the contract is not initialized.
        /// Returns [`SubscriptionError::PlanNotFound`] if no plan exists with the provided ID.
        /// Returns [`SubscriptionError::PlanInactive`] if the selected plan is not active.
        /// Returns [`SubscriptionError::AlreadySubscribed`] if the subscriber already has an active plan.
        pub fn subscribe(
            env: Env,
            subscriber: Address,
            plan_id: Symbol,
            trial_ledgers: Option<u32>,
        ) -> Result<(), SubscriptionError> {
            if !env.storage().instance().has(&DataKey::Provider) {
                return Err(SubscriptionError::NotInitialized);
            }

            // Get the plan details
            let plan_key = DataKey::Plan(plan_id.clone());
            let plan: Plan = env
                .storage()
                .persistent()
                .get(&plan_key)
                .ok_or(SubscriptionError::PlanNotFound)?;

            if !plan.active {
                return Err(SubscriptionError::PlanInactive);
            }

            subscriber.require_auth();

            let sub_key = DataKey::Subscription(subscriber.clone());
            if let Some(existing) = env
                .storage()
                .persistent()
                .get::<_, SubscriptionInfo>(&sub_key)
            {
                if existing.active {
                    return Err(SubscriptionError::AlreadySubscribed);
                }
            }

            let trial_ledgers = trial_ledgers.unwrap_or(0);
            let info = SubscriptionInfo {
                plan_id,
                amount: plan.amount,
                interval_ledgers: plan.interval_ledgers,
                trial_ledgers,
                trial_completed: trial_ledgers == 0,
                last_charged_ledger: env.ledger().sequence(),
                active: true,
            };

            env.storage().persistent().set(&sub_key, &info);
            bump_subscription(&env, &subscriber);
            bump_instance(&env);

            events::subscribed(
                &env,
                &subscriber,
                &info.plan_id,
                plan.amount,
                plan.interval_ledgers,
            );
            Ok(())
        }

        /// Provider pulls a recurring payment from a subscriber.
        ///
        /// Requires the subscriber to have an active subscription and to have granted
        /// sufficient allowance to this contract. The interval since the last charge
        /// must have fully elapsed.
        ///
        /// # Errors
        ///
        /// Returns [`SubscriptionError::NotInitialized`] if the contract is not initialized.
        /// Returns [`SubscriptionError::NotAuthorized`] if the caller is not the provider.
        /// Returns [`SubscriptionError::NotSubscribed`] if no subscription exists for `subscriber`.
        /// Returns [`SubscriptionError::SubscriptionInactive`] if the subscription was cancelled.
        /// Returns [`SubscriptionError::IntervalNotElapsed`] if the charge interval has not passed.
        /// Returns [`SubscriptionError::InsufficientAllowance`] if the subscriber's token allowance is too low.
        pub fn charge(env: Env, subscriber: Address) -> Result<(), SubscriptionError> {
            let provider: Address = env
                .storage()
                .instance()
                .get(&DataKey::Provider)
                .ok_or(SubscriptionError::NotInitialized)?;
            let token_addr: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .ok_or(SubscriptionError::NotInitialized)?;

            provider.require_auth();

            let key = DataKey::Subscription(subscriber.clone());
            let mut info: SubscriptionInfo = env
                .storage()
                .persistent()
                .get(&key)
                .ok_or(SubscriptionError::NotSubscribed)?;

            if !info.active {
                return Err(SubscriptionError::SubscriptionInactive);
            }

            let current_ledger = env.ledger().sequence();

            // Check if trial period is still active
            if !info.trial_completed {
                if current_ledger < info.last_charged_ledger + info.trial_ledgers {
                    return Err(SubscriptionError::IntervalNotElapsed);
                }
                // Trial period completed - mark as completed and update last charged ledger
                info.trial_completed = true;
                info.last_charged_ledger = current_ledger;
                env.storage().persistent().set(&key, &info);
                bump_subscription(&env, &subscriber);
                bump_instance(&env);
                events::trial_completed(&env, &subscriber);
                return Ok(());
            }

            // Normal billing period check
            if current_ledger < info.last_charged_ledger + info.interval_ledgers {
                return Err(SubscriptionError::IntervalNotElapsed);
            }

            let token_client = token::Client::new(&env, &token_addr);

            let allowance = token_client.allowance(&subscriber, &env.current_contract_address());
            if allowance < info.amount {
                return Err(SubscriptionError::InsufficientAllowance);
            }

            // checks-effects-interactions: update state before external call
            info.last_charged_ledger = current_ledger;
            env.storage().persistent().set(&key, &info);
            bump_subscription(&env, &subscriber);
            bump_instance(&env);

            token_client.transfer_from(
                &env.current_contract_address(),
                &subscriber,
                &provider,
                &info.amount,
            );

            events::charged(&env, &subscriber, &provider, info.amount);
            Ok(())
        }

        /// Subscriber cancels their subscription. No further charges can be made.
        ///
        /// # Errors
        ///
        /// Returns [`SubscriptionError::NotInitialized`] if the contract is not initialized.
        /// Returns [`SubscriptionError::NotSubscribed`] if no subscription exists for `subscriber`.
        /// Returns [`SubscriptionError::SubscriptionInactive`] if already cancelled.
        pub fn cancel(env: Env, subscriber: Address) -> Result<(), SubscriptionError> {
            if !env.storage().instance().has(&DataKey::Provider) {
                return Err(SubscriptionError::NotInitialized);
            }

            subscriber.require_auth();

            let key = DataKey::Subscription(subscriber.clone());
            let mut info: SubscriptionInfo = env
                .storage()
                .persistent()
                .get(&key)
                .ok_or(SubscriptionError::NotSubscribed)?;

            if !info.active {
                return Err(SubscriptionError::SubscriptionInactive);
            }

            info.active = false;
            env.storage().persistent().set(&key, &info);
            bump_instance(&env);

            events::cancelled(&env, &subscriber);
            Ok(())
        }

        /// Return the subscription details for `subscriber`, or `None` if not subscribed.
        pub fn get_subscription(env: Env, subscriber: Address) -> Option<SubscriptionInfo> {
            env.storage()
                .persistent()
                .get(&DataKey::Subscription(subscriber))
        }

        /// Return the provider address, or `None` if not initialized.
        pub fn get_provider(env: Env) -> Option<Address> {
            env.storage().instance().get(&DataKey::Provider)
        }

        /// Return the payment token address, or `None` if not initialized.
        pub fn get_token(env: Env) -> Option<Address> {
            env.storage().instance().get(&DataKey::Token)
        }

        /// Return the plan details for the given ID, or `None` if the plan doesn't exist.
        pub fn get_plan(env: Env, plan_id: Symbol) -> Option<Plan> {
            env.storage().persistent().get(&DataKey::Plan(plan_id))
        }
    }
}

#[cfg(test)]
mod test;

#[cfg(test)]
mod prop_test;
