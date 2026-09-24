#![no_std]
#![deny(missing_docs)]
//! Merkle-proof airdrop contract template.
//!
//! An admin sets a merkle root describing the distribution; eligible accounts
//! claim their allocation by presenting a merkle proof. Duplicate claims are
//! rejected on-chain.

use soroban_sdk::{Address, Bytes, BytesN, Env, Vec, contract, contractimpl, token, xdr::ToXdr};

mod errors;
mod events;
mod storage;

pub use errors::AirdropError;
pub use storage::DataKey;

use soroban_common::{LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD};

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

fn bump_claimed(env: &Env, recipient: &Address) {
    env.storage().persistent().extend_ttl(
        &DataKey::Claimed(recipient.clone()),
        LEDGER_LIFETIME_THRESHOLD,
        LEDGER_BUMP_AMOUNT,
    );
}

/// Compute the merkle leaf: sha256(recipient_bytes || amount_be_bytes).
fn compute_leaf(env: &Env, recipient: &Address, amount: i128) -> BytesN<32> {
    let mut data = Bytes::new(env);
    // Encode recipient as its XDR/SC address bytes via to_xdr equivalent.
    // We use the address bytes representation available via soroban's Bytes conversion.
    let addr_bytes = recipient.clone().to_xdr(env);
    data.append(&addr_bytes);
    // Encode amount as 16-byte big-endian.
    let amount_bytes: [u8; 16] = amount.to_be_bytes();
    data.append(&Bytes::from_slice(env, &amount_bytes));
    env.crypto().sha256(&data).into()
}

/// Sort-and-hash two nodes (standard sorted-pair merkle tree).
fn hash_pair(env: &Env, a: &BytesN<32>, b: &BytesN<32>) -> BytesN<32> {
    let mut data = Bytes::new(env);
    // Sort to make the tree order-independent.
    if a.to_array() <= b.to_array() {
        data.append(&Bytes::from(a.clone()));
        data.append(&Bytes::from(b.clone()));
    } else {
        data.append(&Bytes::from(b.clone()));
        data.append(&Bytes::from(a.clone()));
    }
    env.crypto().sha256(&data).into()
}

/// Verify a merkle proof.
///
/// `proof` is the list of sibling hashes from leaf to root.
/// `root` is the expected merkle root.
fn verify_proof(env: &Env, leaf: BytesN<32>, proof: &Vec<BytesN<32>>, root: &BytesN<32>) -> bool {
    let mut current = leaf;
    for sibling in proof.iter() {
        current = hash_pair(env, &current, &sibling);
    }
    &current == root
}

/// Merkle-proof airdrop contract.
///
/// Lifecycle:
/// 1. Admin calls `initialize` to set the token address.
/// 2. Admin calls `set_root` with the merkle root of the airdrop distribution tree.
/// 3. Each eligible address calls `claim(amount, proof)` with a pre-computed merkle proof.
///    Duplicate claims are rejected on-chain.
pub use contract::*;

// The `#[contract]` / `#[contractimpl]` macros generate an undocumented public
// client type. Confine the missing_docs allowance to this module and re-export
// the public contract API above, keeping the rest of the crate enforced.
mod contract {
    #![allow(missing_docs)]
    use super::*;

    #[contract]
    pub struct AirdropContract;

    #[contractimpl]
    impl AirdropContract {
        /// Initialize the airdrop contract.
        ///
        /// # Errors
        ///
        /// Returns [`AirdropError::AlreadyInitialized`] if already initialized.
        pub fn initialize(
            env: Env,
            admin: Address,
            token: Address,
            claim_deadline: u32,
        ) -> Result<(), AirdropError> {
            if env.storage().instance().has(&DataKey::Admin) {
                return Err(AirdropError::AlreadyInitialized);
            }

            admin.require_auth();

            // Validate token interface.
            token::Client::new(&env, &token).decimals();

            env.storage().instance().set(&DataKey::Admin, &admin);
            env.storage().instance().set(&DataKey::Token, &token);
            env.storage()
                .instance()
                .set(&DataKey::ClaimDeadline, &claim_deadline);
            bump_instance(&env);
            Ok(())
        }

        /// Set (or replace) the merkle root. Only the admin may call this.
        ///
        /// # Errors
        ///
        /// Returns [`AirdropError::NotInitialized`] if the contract has not been initialized.
        /// Returns [`AirdropError::Unauthorized`] if caller is not the admin.
        pub fn set_root(env: Env, root: BytesN<32>) -> Result<(), AirdropError> {
            let admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .ok_or(AirdropError::NotInitialized)?;

            admin.require_auth();

            let root_bytes = Bytes::from(root.clone());
            env.storage()
                .instance()
                .set(&DataKey::MerkleRoot, &root_bytes);
            bump_instance(&env);

            events::root_set(&env, &root_bytes);
            Ok(())
        }

        /// Claim tokens by supplying a valid merkle proof.
        ///
        /// The caller must appear in the airdrop tree with exactly `amount` tokens.
        ///
        /// # Errors
        ///
        /// Returns [`AirdropError::NotInitialized`] if not initialized.
        /// Returns [`AirdropError::RootNotSet`] if no merkle root has been set.
        /// Returns [`AirdropError::InvalidAmount`] if `amount <= 0`.
        /// Returns [`AirdropError::ClaimWindowClosed`] if the claim deadline has passed.
        /// Returns [`AirdropError::AlreadyClaimed`] if the address already claimed.
        /// Returns [`AirdropError::InvalidProof`] if the merkle proof does not verify.
        pub fn claim(
            env: Env,
            recipient: Address,
            amount: i128,
            proof: Vec<BytesN<32>>,
        ) -> Result<(), AirdropError> {
            let token_addr: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .ok_or(AirdropError::NotInitialized)?;

            let root_bytes: Bytes = env
                .storage()
                .instance()
                .get(&DataKey::MerkleRoot)
                .ok_or(AirdropError::RootNotSet)?;

            if amount <= 0 {
                return Err(AirdropError::InvalidAmount);
            }

            // Reject claims past the deadline.
            let deadline: u32 = env
                .storage()
                .instance()
                .get(&DataKey::ClaimDeadline)
                .ok_or(AirdropError::NotInitialized)?;
            if env.ledger().sequence() > deadline {
                return Err(AirdropError::ClaimWindowClosed);
            }

            recipient.require_auth();

            // Duplicate-claim prevention.
            let claimed_key = DataKey::Claimed(recipient.clone());
            if env
                .storage()
                .persistent()
                .get::<_, bool>(&claimed_key)
                .unwrap_or(false)
            {
                return Err(AirdropError::AlreadyClaimed);
            }

            // Convert stored bytes back to BytesN<32>.
            let root: BytesN<32> = root_bytes
                .try_into()
                .map_err(|_| AirdropError::RootNotSet)?;

            let leaf = compute_leaf(&env, &recipient, amount);
            if !verify_proof(&env, leaf, &proof, &root) {
                return Err(AirdropError::InvalidProof);
            }

            // Checks-effects-interactions: mark claimed before transfer.
            env.storage().persistent().set(&claimed_key, &true);
            bump_claimed(&env, &recipient);
            bump_instance(&env);

            token::Client::new(&env, &token_addr).transfer(
                &env.current_contract_address(),
                &recipient,
                &amount,
            );

            events::claimed(&env, &recipient, amount);
            Ok(())
        }

        /// Batch-claim tokens for multiple recipients in a single transaction.
        ///
        /// A relayer (or the recipients themselves) may submit a list of
        /// `(recipient, amount, proof)` tuples. The function uses **all-or-nothing**
        /// semantics: if any entry fails proof verification or has already been
        /// claimed, the entire transaction is aborted and no tokens are transferred.
        ///
        /// # Errors
        ///
        /// Returns [`AirdropError::NotInitialized`] if not initialized.
        /// Returns [`AirdropError::RootNotSet`] if no merkle root has been set.
        /// Returns [`AirdropError::ClaimWindowClosed`] if the claim deadline has passed.
        /// Returns [`AirdropError::InvalidAmount`] if any entry has `amount <= 0`.
        /// Returns [`AirdropError::AlreadyClaimed`] if any entry has already been claimed.
        /// Returns [`AirdropError::InvalidProof`] if any entry's merkle proof is invalid.
        pub fn claim_batch(
            env: Env,
            entries: Vec<(Address, i128, Vec<BytesN<32>>)>,
        ) -> Result<(), AirdropError> {
            let token_addr: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .ok_or(AirdropError::NotInitialized)?;

            let root_bytes: Bytes = env
                .storage()
                .instance()
                .get(&DataKey::MerkleRoot)
                .ok_or(AirdropError::RootNotSet)?;

            // Deadline check applies to the whole batch.
            let deadline: u32 = env
                .storage()
                .instance()
                .get(&DataKey::ClaimDeadline)
                .ok_or(AirdropError::NotInitialized)?;
            if env.ledger().sequence() > deadline {
                return Err(AirdropError::ClaimWindowClosed);
            }

            let root: BytesN<32> = root_bytes
                .try_into()
                .map_err(|_| AirdropError::RootNotSet)?;

            // --- Validate all entries before any state change (all-or-nothing) ---
            for (recipient, amount, proof) in entries.iter() {
                if amount <= 0 {
                    return Err(AirdropError::InvalidAmount);
                }
                let claimed_key = DataKey::Claimed(recipient.clone());
                if env
                    .storage()
                    .persistent()
                    .get::<_, bool>(&claimed_key)
                    .unwrap_or(false)
                {
                    return Err(AirdropError::AlreadyClaimed);
                }
                let leaf = compute_leaf(&env, &recipient, amount);
                if !verify_proof(&env, leaf, &proof, &root) {
                    return Err(AirdropError::InvalidProof);
                }
            }

            // --- Apply state changes and transfers ---
            for (recipient, amount, _proof) in entries.iter() {
                let claimed_key = DataKey::Claimed(recipient.clone());
                env.storage().persistent().set(&claimed_key, &true);
                bump_claimed(&env, &recipient);

                token::Client::new(&env, &token_addr).transfer(
                    &env.current_contract_address(),
                    &recipient,
                    &amount,
                );

                events::claimed(&env, &recipient, amount);
            }

            bump_instance(&env);
            Ok(())
        }

        /// Returns `true` if `address` has already claimed.
        pub fn is_claimed(env: Env, address: Address) -> bool {
            env.storage()
                .persistent()
                .get::<_, bool>(&DataKey::Claimed(address))
                .unwrap_or(false)
        }

        /// Returns the current merkle root, or `None` if not set.
        pub fn get_root(env: Env) -> Option<Bytes> {
            env.storage().instance().get(&DataKey::MerkleRoot)
        }
    }
}

#[cfg(test)]
mod test;

#[cfg(test)]
mod prop_test;
