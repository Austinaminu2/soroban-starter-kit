#![no_std]
#![deny(missing_docs)]
//! Non-fungible token (NFT) contract template.
//!
//! Admin-controlled minting with per-token owners, approved spenders, metadata
//! URIs, and an optional supply cap.

use soroban_sdk::{Address, Env, String, Vec, contract, contractimpl};

mod errors;
mod events;
mod storage;

pub use errors::NftError;
pub use storage::{DataKey, RoyaltyInfo, TokenKey, TokenMetadata};

use soroban_common::{LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD, apply_bps_fee};

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

/// Non-fungible token (NFT) contract.
pub use contract::*;

mod contract {
    #![allow(missing_docs)]
    use super::*;
    use storage::DataKey::*;
    use storage::{RoyaltyInfo, TokenKey, TokenMetadata};

    #[contract]
    pub struct NftContract;

    #[contractimpl]
    impl NftContract {
        /// Initialise the NFT contract.
        ///
        /// `royalty_bps` sets a collection-level default royalty in basis points
        /// (0–10 000). `royalty_recipient` is required when `royalty_bps > 0`.
        /// Both default to no royalty when omitted (`None`).
        ///
        /// # Errors
        /// - [`NftError::AlreadyInitialized`]
        /// - [`NftError::InvalidRoyalty`] if `royalty_bps > 10 000`.
        /// - [`NftError::RoyaltyRecipientMissing`] if `royalty_bps > 0` but
        ///   `royalty_recipient` is `None`.
        pub fn initialize(
            env: Env,
            admin: Address,
            name: String,
            symbol: String,
            max_supply: Option<u32>,
            royalty_bps: Option<u32>,
            royalty_recipient: Option<Address>,
        ) -> Result<(), NftError> {
            if env.storage().instance().has(&State) {
                return Err(NftError::AlreadyInitialized);
            }
            // Validate collection-level royalty parameters.
            if let Some(bps) = royalty_bps {
                if bps > 10_000 {
                    return Err(NftError::InvalidRoyalty);
                }
                if bps > 0 && royalty_recipient.is_none() {
                    return Err(NftError::RoyaltyRecipientMissing);
                }
            }
            admin.require_auth();

            env.storage().instance().set(&Admin, &admin);
            env.storage().instance().set(&Name, &name);
            env.storage().instance().set(&Symbol, &symbol);
            env.storage()
                .instance()
                .set(&DataKey::TotalSupply, &0u32);
            if let Some(supply) = max_supply {
                env.storage().instance().set(&DataKey::MaxSupply, &supply);
            }
            // Store collection-level royalty parameters if provided.
            if let Some(bps) = royalty_bps {
                env.storage().instance().set(&DataKey::RoyaltyBps, &bps);
            }
            if let Some(ref recipient) = royalty_recipient {
                env.storage()
                    .instance()
                    .set(&DataKey::RoyaltyRecipient, recipient);
            }
            env.storage().instance().set(&State, &true);

            bump_instance(&env);
            events::initialized(&env, &admin, name, symbol);
            Ok(())
        }

        /// Mint a new NFT to `to`.
        ///
        /// `token_royalty_bps` and `token_royalty_recipient` are optional per-token
        /// royalty overrides (EIP-2981-style). When set they take precedence over the
        /// collection-level defaults returned by [`royalty_info`]. Both must be `None`
        /// or both must be `Some` (with `token_royalty_bps` in 0–10 000).
        ///
        /// Returns the newly minted `token_id`.
        ///
        /// # Errors
        /// - [`NftError::NotInitialized`]
        /// - [`NftError::Unauthorized`]
        /// - [`NftError::MaxSupplyReached`]
        /// - [`NftError::InvalidRoyalty`] if `token_royalty_bps > 10 000`.
        /// - [`NftError::RoyaltyRecipientMissing`] if `token_royalty_bps > 0` but
        ///   `token_royalty_recipient` is `None`.
        pub fn mint(
            env: Env,
            to: Address,
            token_uri: String,
            token_royalty_bps: Option<u32>,
            token_royalty_recipient: Option<Address>,
        ) -> Result<u32, NftError> {
            let admin: Address = env.storage().instance().get(&Admin).unwrap();
            admin.require_auth();

            let total: u32 = env
                .storage()
                .instance()
                .get(&DataKey::TotalSupply)
                .unwrap_or(0);
            let token_id = total.checked_add(1).ok_or(NftError::MaxSupplyReached)?;

            if let Some(max) = env
                .storage()
                .instance()
                .get::<_, u32>(&DataKey::MaxSupply)
            {
                if token_id > max {
                    return Err(NftError::MaxSupplyReached);
                }
            }

            // Validate per-token royalty parameters.
            if let Some(bps) = token_royalty_bps {
                if bps > 10_000 {
                    return Err(NftError::InvalidRoyalty);
                }
                if bps > 0 && token_royalty_recipient.is_none() {
                    return Err(NftError::RoyaltyRecipientMissing);
                }
            } else if token_royalty_recipient.is_some() {
                return Err(NftError::InvalidRoyalty);
            }

            env.storage()
                .instance()
                .set(&DataKey::TotalSupply, &token_id);
            env.storage()
                .instance()
                .set(&TokenKey::Owner(token_id), &to);
            env.storage()
                .instance()
                .set(&TokenKey::TokenUri(token_id), &token_uri);

            // Store per-token royalty overrides if provided.
            if let Some(bps) = token_royalty_bps {
                env.storage()
                    .instance()
                    .set(&TokenKey::TokenRoyaltyBps(token_id), &bps);
            }
            if let Some(ref recipient) = token_royalty_recipient {
                env.storage().instance().set(
                    &TokenKey::TokenRoyaltyRecipient(token_id),
                    recipient,
                );
            }

            bump_instance(&env);
            events::minted(&env, &to, token_id);
            Ok(token_id)
        }

        /// Return royalty information for a token sale (EIP-2981-style view).
        ///
        /// Resolution order:
        /// 1. Per-token royalty BPS and recipient (set at mint time), if present.
        /// 2. Collection-level royalty BPS and recipient (set at initialize time).
        /// 3. No royalty (`None`).
        ///
        /// [`RoyaltyInfo::amount`] is pre-computed as `sale_price * bps / 10_000`.
        ///
        /// # Errors
        /// - [`NftError::NotInitialized`]
        /// - [`NftError::TokenNotFound`] if `token_id` does not exist
        pub fn royalty_info(
            env: Env,
            token_id: u32,
            sale_price: i128,
        ) -> Result<Option<RoyaltyInfo>, NftError> {
            // 1. Per-token overrides.
            let token_bps: Option<u32> = env
                .storage()
                .instance()
                .get(&TokenKey::TokenRoyaltyBps(token_id));
            let token_recipient: Option<Address> = env
                .storage()
                .persistent()
                .get(&TokenKey::TokenRoyaltyRecipient(token_id));

            // A per-token override with `bps == 0` legitimately has no recipient
            // (`mint` only requires a recipient when `bps > 0`), so the override
            // applies whenever `token_bps` is present, not only when a recipient
            // is present too.
            if let Some(bps) = token_bps {
                if bps == 0 {
                    return Ok(None);
                }
                #[allow(
                    clippy::arithmetic_side_effects,
                    clippy::as_conversions,
                    clippy::cast_possible_truncation,
                    clippy::integer_division
                )]
                let amount = (sale_price * bps as i128) / 10_000;
                let recipient = token_recipient.ok_or(NftError::RoyaltyRecipientMissing)?;
                let amount = apply_bps_fee(sale_price, bps).unwrap_or(0);
                return Ok(Some(RoyaltyInfo { recipient, amount }));
            }

            // 2. Collection-level defaults.
            let collection_bps: Option<u32> = env.storage().instance().get(&DataKey::RoyaltyBps);
            let collection_recipient: Option<Address> =
                env.storage().instance().get(&DataKey::RoyaltyRecipient);

            if let (Some(bps), Some(recipient)) = (collection_bps, collection_recipient) {
                if bps == 0 {
                    return Ok(None);
                }
                #[allow(
                    clippy::arithmetic_side_effects,
                    clippy::as_conversions,
                    clippy::cast_possible_truncation,
                    clippy::integer_division
                )]
                let amount = (sale_price * bps as i128) / 10_000;
                let amount = apply_bps_fee(sale_price, bps).unwrap_or(0);
                return Ok(Some(RoyaltyInfo { recipient, amount }));
            }

            // 3. No royalty configured.
            Ok(None)
        }

        /// Transfer token `token_id` from `from` to `to`. Requires auth from `from` (the owner).
        ///
        /// # Errors
        /// - [`NftError::NotInitialized`]
        /// - [`NftError::TokenNotFound`]
        /// - [`NftError::Unauthorized`]
        pub fn transfer(
            env: Env,
            from: Address,
            to: Address,
            token_id: u32,
        ) -> Result<(), NftError> {
            from.require_auth();

            let owner: Address = env
                .storage()
                .instance()
                .get(&TokenKey::Owner(token_id))
                .ok_or(NftError::TokenNotFound)?;
            if owner != from {
                return Err(NftError::Unauthorized);
            }

            env.storage()
                .instance()
                .set(&TokenKey::Owner(token_id), &to);
            bump_instance(&env);
            events::transferred(&env, &from, &to, token_id);
            Ok(())
        }

        /// Approve `spender` to transfer or sell token `token_id` on behalf of the owner.
        ///
        /// # Errors
        /// - [`NftError::NotInitialized`]
        /// - [`NftError::TokenNotFound`]
        /// - [`NftError::Unauthorized`]
        pub fn approve(
            env: Env,
            owner: Address,
            spender: Address,
            token_id: u32,
        ) -> Result<(), NftError> {
            owner.require_auth();

            let current_owner: Address = env
                .storage()
                .instance()
                .get(&TokenKey::Owner(token_id))
                .ok_or(NftError::TokenNotFound)?;
            if current_owner != owner {
                return Err(NftError::Unauthorized);
            }

            env.storage().instance().set(
                &TokenKey::Approved(token_id),
                &spender,
            );
            bump_instance(&env);
            events::approved(&env, &owner, &spender, token_id);
            Ok(())
        }

        /// Get the owner of `token_id`.
        ///
        /// # Errors
        /// - [`NftError::NotInitialized`]
        /// - [`NftError::TokenNotFound`]
        pub fn owner_of(env: Env, token_id: u32) -> Result<Address, NftError> {
            env.storage()
                .instance()
                .get(&TokenKey::Owner(token_id))
                .ok_or(NftError::TokenNotFound)
        }

        /// Get the metadata URI of `token_id`.
        ///
        /// # Errors
        /// - [`NftError::NotInitialized`]
        /// - [`NftError::TokenNotFound`]
        pub fn token_uri(env: Env, token_id: u32) -> Result<String, NftError> {
            env.storage()
                .instance()
                .get(&TokenKey::TokenUri(token_id))
                .ok_or(NftError::TokenNotFound)
        }

        /// Get total supply.
        pub fn total_supply(env: Env) -> u32 {
            env.storage()
                .instance()
                .get(&DataKey::TotalSupply)
                .unwrap_or(0)
        }

        /// Get the collection-level royalty BPS.
        pub fn get_royalty_bps(env: Env) -> Option<u32> {
            env.storage().instance().get(&DataKey::RoyaltyBps)
        }

        /// Get the collection-level royalty recipient.
        pub fn get_royalty_recipient(env: Env) -> Option<Address> {
            env.storage().instance().get(&DataKey::RoyaltyRecipient)
        }
    }
}
