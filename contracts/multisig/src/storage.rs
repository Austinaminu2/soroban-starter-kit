// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, Map, Symbol, Val, Vec, contracttype};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Signers,
    /// Per-signer vote weights. Key: signer address, value: weight (u32).
    /// When absent all signers have weight 1 (backward-compatible default).
    Weights,
    /// Accumulated weight threshold required to execute a proposal.
    /// For un-weighted wallets this equals the signer count threshold.
    Threshold,
    NextTransactionId,
    Transaction(u64),
    Paused,
    Version,
    /// Next ID to assign to an asynchronous signer-management proposal (#1112).
    NextSignerProposalId,
    /// Asynchronous signer-management proposal keyed by its ID (#1112).
    SignerProposal(u64),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transaction {
    pub id: u64,
    pub proposer: Address,
    pub target: Address,
    pub function: Symbol,
    pub args: Vec<Val>,
    /// Addresses that have signed this proposal.
    pub signatures: Vec<Address>,
    /// Accumulated weight of all signatures collected so far.
    pub accumulated_weight: u32,
    pub executed: bool,
    /// Ledger sequence after which this proposal is considered expired.
    pub expiry_ledger: u32,
}

/// Per-signer weight entry stored in the `Weights` map.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignerWeight {
    pub signer: Address,
    pub weight: u32,
}

/// A change to the signer set, threshold, or signer weights that can be
/// proposed and approved asynchronously (#1112).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignerChange {
    /// Add `SignerWeight.signer` with `SignerWeight.weight` and set the new threshold.
    AddSigner(SignerWeight, u32),
    /// Remove the signer and set the new threshold.
    RemoveSigner(Address, u32),
    /// Change an existing signer's weight; the threshold is unchanged.
    UpdateSignerWeight(SignerWeight),
    /// Change the accumulated-weight threshold.
    ChangeThreshold(u32),
}

/// A pending signer-management proposal collecting signatures across ledgers (#1112).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignerProposal {
    pub id: u64,
    pub proposer: Address,
    pub change: SignerChange,
    /// Addresses that have signed this proposal.
    pub signatures: Vec<Address>,
    /// Accumulated weight of all signatures collected so far.
    pub accumulated_weight: u32,
    pub executed: bool,
    /// Ledger sequence after which this proposal is considered expired.
    pub expiry_ledger: u32,
}

/// Retrieve the weight assigned to `signer`.  Returns 1 if no weights map is
/// stored (i.e. the wallet was initialized without per-signer weights).
pub fn get_weight(weights: &Map<Address, u32>, signer: &Address) -> u32 {
    weights.get(signer.clone()).unwrap_or(1)
}
