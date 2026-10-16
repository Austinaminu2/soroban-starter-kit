use soroban_sdk::{Address, Env, Vec, token};

use crate::admin;
use crate::errors::EscrowError;
use crate::events;
use crate::storage::{DataKey, EscrowState, Milestone, require_state};
use soroban_common::{
    LEDGER_BUMP_AMOUNT, LEDGER_LIFETIME_THRESHOLD, extend_ttl_instance, validate_deadline,
};

use DataKey::*;

pub fn get_required<T: soroban_sdk::TryFromVal<soroban_sdk::Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
) -> Result<T, EscrowError> {
    env.storage()
        .instance()
        .get(key)
        .ok_or(EscrowError::NotInitialized)
}

fn validate_amount(amount: i128) -> Result<(), EscrowError> {
    if amount <= 0 {
        return Err(EscrowError::InvalidAmount);
    }
    Ok(())
}

fn validate_parties(
    buyer: &Address,
    seller: &Address,
    arbiter: &Address,
) -> Result<(), EscrowError> {
    if buyer == seller || buyer == arbiter || seller == arbiter {
        return Err(EscrowError::InvalidParties);
    }
    Ok(())
}

fn validate_parties_multi(
    buyer: &Address,
    seller: &Address,
    arbiters: &Vec<Address>,
    required_signatures: u32,
) -> Result<(), EscrowError> {
    if arbiters.is_empty()
        || required_signatures == 0
        || required_signatures > arbiters.len() as u32
    {
        return Err(EscrowError::InvalidParties);
    }
    for arbiter in arbiters.iter() {
        if &arbiter == buyer || &arbiter == seller {
            return Err(EscrowError::InvalidParties);
        }
    }
    Ok(())
}

pub(crate) fn extend_ttl(env: &Env) {
    extend_ttl_instance(env, LEDGER_LIFETIME_THRESHOLD, LEDGER_BUMP_AMOUNT);
}

fn store_escrow_data(
    env: &Env,
    buyer: &Address,
    seller: &Address,
    arbiter: &Address,
    token_contract: &Address,
    amount: i128,
    deadline_ledger: u32,
    required_signatures: u32,
    dispute_timeout_ledgers: u32,
) {
    env.storage().instance().set(&Buyer, buyer);
    env.storage().instance().set(&Seller, seller);
    env.storage().instance().set(&Arbiter, arbiter);
    env.storage().instance().set(&TokenContract, token_contract);
    env.storage().instance().set(&Amount, &amount);
    env.storage().instance().set(&Deadline, &deadline_ledger);
    env.storage().instance().set(&State, &EscrowState::Created);
    env.storage()
        .instance()
        .set(&RequiredSignatures, &required_signatures);
    env.storage().instance().set(&Version, &2u32);
    env.storage()
        .instance()
        .set(&DisputeTimeoutLedgers, &dispute_timeout_ledgers);
}

fn emit_init_events(env: &Env, buyer: &Address, seller: &Address, arbiter: &Address, amount: i128) {
    events::escrow_created(env, buyer, seller, amount);
    events::initialized(env, buyer, seller, arbiter, amount);
}

#[allow(clippy::too_many_arguments)]
pub fn initialize(
    env: Env,
    admin: Address,
    buyer: Address,
    seller: Address,
    arbiter: Address,
    token_contract: Address,
    amount: i128,
    deadline_ledger: u32,
    dispute_timeout_ledgers: u32,
    metadata_hash: Option<soroban_sdk::BytesN<32>>,
) -> Result<(), EscrowError> {
    if env.storage().instance().has(&State) {
        return Err(EscrowError::AlreadyInitialized);
    }
    validate_amount(amount)?;
    validate_parties(&buyer, &seller, &arbiter)?;
    validate_deadline::<EscrowError>(&env, deadline_ledger)?;
    token::Client::new(&env, &token_contract).decimals();
    env.storage()
        .instance()
        .set(&soroban_common::AdminKey::Admin, &admin);
    store_escrow_data(
        &env,
        &buyer,
        &seller,
        &arbiter,
        &token_contract,
        amount,
        deadline_ledger,
        1u32,
        dispute_timeout_ledgers,
    );
    if let Some(hash) = metadata_hash {
        env.storage().instance().set(&DataKey::MetadataHash, &hash);
    }
    extend_ttl(&env);
    emit_init_events(&env, &buyer, &seller, &arbiter, amount);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn initialize_with_arbiters(
    env: Env,
    admin: Address,
    buyer: Address,
    seller: Address,
    arbiters: soroban_sdk::Vec<Address>,
    token_contract: Address,
    amount: i128,
    deadline_ledger: u32,
    required_signatures: u32,
    dispute_timeout_ledgers: u32,
    metadata_hash: Option<soroban_sdk::BytesN<32>>,
) -> Result<(), EscrowError> {
    if env.storage().instance().has(&State) {
        return Err(EscrowError::AlreadyInitialized);
    }
    validate_amount(amount)?;
    validate_parties_multi(&buyer, &seller, &arbiters, required_signatures)?;
    validate_deadline::<EscrowError>(&env, deadline_ledger)?;
    token::Client::new(&env, &token_contract).decimals();
    env.storage()
        .instance()
        .set(&soroban_common::AdminKey::Admin, &admin);
    #[allow(clippy::unwrap_used)] // arbiters is validated non-empty before this point
    let primary_arbiter = arbiters.get(0).unwrap();
    store_escrow_data(
        &env,
        &buyer,
        &seller,
        &primary_arbiter,
        &token_contract,
        amount,
        deadline_ledger,
        required_signatures,
        dispute_timeout_ledgers,
    );
    env.storage().instance().set(&Arbiters, &arbiters);
    if let Some(hash) = metadata_hash {
        env.storage().instance().set(&DataKey::MetadataHash, &hash);
    }
    extend_ttl(&env);
    emit_init_events(&env, &buyer, &seller, &primary_arbiter, amount);
    Ok(())
}

pub fn update_amount(env: Env, new_amount: i128) -> Result<(), EscrowError> {
    let buyer: Address = get_required(&env, &Buyer)?;
    buyer.require_auth();

    if new_amount <= 0 {
        return Err(EscrowError::InvalidAmount);
    }

    let state: EscrowState = get_required(&env, &State)?;
    if state != EscrowState::Created {
        return Err(EscrowError::InvalidState);
    }

    env.storage().instance().set(&Amount, &new_amount);
    extend_ttl(&env);

    events::amount_updated(&env, &buyer, new_amount);

    Ok(())
}

pub fn fund(env: Env) -> Result<(), EscrowError> {
    #[cfg(feature = "pausable")]
    crate::EscrowContract::require_not_paused(&env)?;

    let buyer: Address = get_required(&env, &Buyer)?;
    buyer.require_auth();

    let state: EscrowState = get_required(&env, &State)?;
    if state != EscrowState::Created {
        return Err(EscrowError::InvalidState);
    }

    let token_contract: Address = get_required(&env, &TokenContract)?;
    let amount: i128 = get_required(&env, &Amount)?;

    let token_client = token::Client::new(&env, &token_contract);
    if token_client.balance(&buyer) < amount {
        return Err(EscrowError::InsufficientFunds);
    }
    token_client.transfer(&buyer, &env.current_contract_address(), &amount);

    env.storage().instance().set(&State, &EscrowState::Funded);
    extend_ttl(&env);

    events::escrow_funded(&env, &buyer, amount);

    Ok(())
}

pub fn mark_delivered(env: Env) -> Result<(), EscrowError> {
    #[cfg(feature = "pausable")]
    crate::EscrowContract::require_not_paused(&env)?;

    let seller: Address = get_required(&env, &Seller)?;
    seller.require_auth();

    let state: EscrowState = get_required(&env, &State)?;
    if state != EscrowState::Funded {
        return Err(EscrowError::InvalidState);
    }

    env.storage()
        .instance()
        .set(&State, &EscrowState::Delivered);
    extend_ttl(&env);

    events::delivery_marked(&env, &seller);

    Ok(())
}

pub fn approve_delivery(env: Env) -> Result<(), EscrowError> {
    #[cfg(feature = "pausable")]
    crate::EscrowContract::require_not_paused(&env)?;

    let buyer: Address = get_required(&env, &Buyer)?;
    buyer.require_auth();

    release_to_seller(env)
}

pub fn release_partial(env: Env, amount: i128) -> Result<(), EscrowError> {
    #[cfg(feature = "pausable")]
    crate::EscrowContract::require_not_paused(&env)?;

    let buyer: Address = get_required(&env, &Buyer)?;
    buyer.require_auth();

    let state: EscrowState = get_required(&env, &State)?;
    if state != EscrowState::Funded {
        return Err(EscrowError::InvalidState);
    }

    if amount <= 0 {
        return Err(EscrowError::InvalidAmount);
    }

    let stored_amount: i128 = get_required(&env, &Amount)?;
    if amount > stored_amount {
        return Err(EscrowError::InsufficientFunds);
    }

    let seller: Address = get_required(&env, &Seller)?;
    let new_amount = stored_amount - amount;
    env.storage().instance().set(&Amount, &new_amount);
    extend_ttl(&env);

    let (net, fee) = apply_fee(&env, amount);
    admin::transfer_token(&env, &env.current_contract_address(), &seller, net);
    maybe_pay_fee(&env, fee);
    events::partial_release(&env, &seller, net, fee);

    Ok(())
}

pub fn request_refund(env: Env) -> Result<(), EscrowError> {
    #[cfg(feature = "pausable")]
    crate::EscrowContract::require_not_paused(&env)?;

    let buyer: Address = get_required(&env, &Buyer)?;
    buyer.require_auth();

    let state: EscrowState = get_required(&env, &State)?;
    let deadline: u32 = get_required(&env, &Deadline)?;

    let can_refund = matches!(state, EscrowState::Funded | EscrowState::Delivered)
        && env.ledger().sequence() > deadline;
    if !can_refund {
        return Err(EscrowError::DeadlineNotReached);
    }

    refund_to_buyer(env)
}

/// Refund the remaining escrow balance to the buyer at any point while in
/// `Funded` state.  This is useful when the buyer has already released a
/// partial amount via [`release_partial`] and wants the unused remainder
/// back without waiting for the deadline (issue #713).
pub fn request_partial_refund(env: Env) -> Result<(), EscrowError> {
    #[cfg(feature = "pausable")]
    crate::EscrowContract::require_not_paused(&env)?;

    let buyer: Address = get_required(&env, &Buyer)?;
    buyer.require_auth();

    let state: EscrowState = get_required(&env, &State)?;
    if state != EscrowState::Funded {
        return Err(EscrowError::InvalidState);
    }

    let amount: i128 = get_required(&env, &Amount)?;
    if amount <= 0 {
        return Err(EscrowError::InvalidAmount);
    }

    refund_to_buyer(env)
}

pub fn cancel(env: Env) -> Result<(), EscrowError> {
    let buyer: Address = get_required(&env, &Buyer)?;
    buyer.require_auth();

    let state: EscrowState = get_required(&env, &State)?;
    if state != EscrowState::Created {
        return Err(EscrowError::InvalidState);
    }

    env.storage()
        .instance()
        .set(&State, &EscrowState::Cancelled);
    extend_ttl(&env);

    events::escrow_cancelled(&env, &buyer);

    Ok(())
}

pub fn extend_deadline(env: Env, new_deadline: u32) -> Result<(), EscrowError> {
    let buyer: Address = get_required(&env, &Buyer)?;
    let seller: Address = get_required(&env, &Seller)?;

    buyer.require_auth();
    seller.require_auth();

    let current_deadline: u32 = get_required(&env, &Deadline)?;
    let current_ledger: u32 = env.ledger().sequence();

    if new_deadline < current_ledger + soroban_common::MIN_DEADLINE_BUFFER {
        return Err(EscrowError::DeadlinePassed);
    }

    if new_deadline <= current_deadline {
        return Err(EscrowError::DeadlinePassed);
    }

    let state: EscrowState = get_required(&env, &State)?;
    if !matches!(state, EscrowState::Funded | EscrowState::Delivered) {
        return Err(EscrowError::InvalidState);
    }

    env.storage().instance().set(&Deadline, &new_deadline);
    extend_ttl(&env);

    events::deadline_extended(&env, &buyer, new_deadline);

    Ok(())
}

/// Remaining funds eligible for release/refund/dispute resolution.
///
/// For multi-milestone escrows the disputable pool is the sum of **unreleased**
/// milestone amounts (already-released milestones are excluded). The stored
/// [`Amount`] is synced to that remainder so payouts cannot exceed what is
/// still held in the contract.
fn sync_disputable_amount(env: &Env) -> Result<i128, EscrowError> {
    let stored: i128 = get_required(env, &Amount)?;
    let Some(milestones) = env
        .storage()
        .instance()
        .get::<DataKey, Vec<Milestone>>(&DataKey::Milestones)
    else {
        return Ok(stored);
    };

    let mut unreleased: i128 = 0;
    for m in milestones.iter() {
        if !m.released {
            unreleased = unreleased
                .checked_add(m.amount)
                .ok_or(EscrowError::InsufficientFunds)?;
        }
    }

    if unreleased != stored {
        env.storage().instance().set(&Amount, &unreleased);
    }
    Ok(unreleased)
}

/// Mark every remaining milestone as released after a full settlement payout.
fn mark_all_milestones_released(env: &Env) {
    let Some(mut milestones) = env
        .storage()
        .instance()
        .get::<DataKey, Vec<Milestone>>(&DataKey::Milestones)
    else {
        return;
    };
    for i in 0..milestones.len() {
        if let Some(mut m) = milestones.get(i) {
            if !m.released {
                m.released = true;
                milestones.set(i, m);
            }
        }
    }
    env.storage()
        .instance()
        .set(&DataKey::Milestones, &milestones);
}

pub fn release_to_seller(env: Env) -> Result<(), EscrowError> {
    require_state(&env, EscrowState::Delivered)?;

    let seller: Address = get_required(&env, &Seller)?;
    // Only redistribute unreleased remainder (issue #1189).
    let amount: i128 = sync_disputable_amount(&env)?;

    env.storage()
        .instance()
        .set(&State, &EscrowState::Completed);
    env.storage().instance().set(&Amount, &0i128);
    mark_all_milestones_released(&env);
    extend_ttl(&env);

    if amount > 0 {
        let (net, fee) = apply_fee(&env, amount);
        admin::transfer_token(&env, &env.current_contract_address(), &seller, net);
        maybe_pay_fee(&env, fee);
        events::funds_released(&env, &seller, net, fee);
    } else {
        events::funds_released(&env, &seller, 0, 0);
    }

    Ok(())
}

pub fn refund_to_buyer(env: Env) -> Result<(), EscrowError> {
    require_state(&env, EscrowState::Funded)?;

    let buyer: Address = get_required(&env, &Buyer)?;
    // Only redistribute unreleased remainder (issue #1189).
    let amount: i128 = sync_disputable_amount(&env)?;

    env.storage().instance().set(&State, &EscrowState::Refunded);
    env.storage().instance().set(&Amount, &0i128);
    mark_all_milestones_released(&env);
    extend_ttl(&env);

    if amount > 0 {
        admin::transfer_token(&env, &env.current_contract_address(), &buyer, amount);
    }

    events::funds_refunded(&env, &buyer, amount);

    Ok(())
}

// ─── Fee configuration ────────────────────────────────────────────────────────

/// Set (or update) the fee configuration.  The caller must be the escrow's
/// platform admin (the address passed to `initialize*` and stored as
/// [`soroban_common::AdminKey::Admin`]) — deliberately not the buyer or
/// seller, so that neither party to a specific deal can unilaterally waive
/// or alter the platform's fee.
///
/// `fee_bps` is in basis points (0 = no fee, 10 000 = 100 %).  Any value
/// greater than 10 000 returns [`EscrowError::FeeTooHigh`].
pub fn set_fee_config(env: Env, fee_bps: u32, treasury: Address) -> Result<(), EscrowError> {
    // Must be initialized.
    if !env.storage().instance().has(&DataKey::State) {
        return Err(EscrowError::NotInitialized);
    }
    let admin = admin::require_admin(&env)?;
    admin.require_auth();

    if fee_bps > 10_000 {
        return Err(EscrowError::FeeTooHigh);
    }
    env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
    env.storage().instance().set(&DataKey::Treasury, &treasury);
    extend_ttl(&env);
    events::fee_config_set(&env, &admin, fee_bps, &treasury);
    Ok(())
}

/// Compute and apply the fee on a gross release amount.
///
/// Returns `(net_to_seller, fee_amount)`.
fn apply_fee(env: &Env, gross: i128) -> (i128, i128) {
    let fee_bps: u32 = env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0);
    if fee_bps == 0 {
        return (gross, 0);
    }
    // fee = gross * fee_bps / 10_000  (integer division — intentional)
    #[allow(
        clippy::integer_division,
        clippy::arithmetic_side_effects,
        clippy::as_conversions,
        clippy::cast_possible_truncation
    )]
    let fee = (gross * fee_bps as i128) / 10_000;
    #[allow(clippy::arithmetic_side_effects)]
    let net = gross - fee;
    (net, fee)
}

/// Route the fee to the treasury if non-zero.
fn maybe_pay_fee(env: &Env, fee: i128) {
    if fee <= 0 {
        return;
    }
    if let Some(treasury) = env
        .storage()
        .instance()
        .get::<DataKey, Address>(&DataKey::Treasury)
    {
        admin::transfer_token(env, &env.current_contract_address(), &treasury, fee);
    }
}

// ─── Multi-milestone support ──────────────────────────────────────────────────

/// Initialize an escrow with explicit milestones.
///
/// The `milestones` list must be non-empty.  The `amount` parameter is
/// ignored; the total escrowed amount is derived from the sum of all milestone
/// amounts.  All other parameters are identical to [`initialize`].
///
/// The buyer funds the escrow with the total amount via the regular [`fund`]
/// call once initialized.
#[allow(clippy::too_many_arguments)]
pub fn initialize_with_milestones(
    env: Env,
    admin: Address,
    buyer: Address,
    seller: Address,
    arbiter: Address,
    token_contract: Address,
    milestones: Vec<Milestone>,
    deadline_ledger: u32,
    dispute_timeout_ledgers: u32,
    metadata_hash: Option<soroban_sdk::BytesN<32>>,
) -> Result<(), EscrowError> {
    if env.storage().instance().has(&DataKey::State) {
        return Err(EscrowError::AlreadyInitialized);
    }
    if milestones.is_empty() {
        return Err(EscrowError::InvalidAmount);
    }
    // Validate milestone amounts and sum them.
    let mut total: i128 = 0i128;
    for m in milestones.iter() {
        if m.amount <= 0 {
            return Err(EscrowError::InvalidAmount);
        }
        total = total
            .checked_add(m.amount)
            .ok_or(EscrowError::InsufficientFunds)?;
    }
    validate_amount(total)?;
    validate_parties(&buyer, &seller, &arbiter)?;
    validate_deadline::<EscrowError>(&env, deadline_ledger)?;
    token::Client::new(&env, &token_contract).decimals();
    env.storage()
        .instance()
        .set(&soroban_common::AdminKey::Admin, &admin);

    store_escrow_data(
        &env,
        &buyer,
        &seller,
        &arbiter,
        &token_contract,
        total,
        deadline_ledger,
        1u32,
        dispute_timeout_ledgers,
    );
    env.storage()
        .instance()
        .set(&DataKey::Milestones, &milestones);
    if let Some(hash) = metadata_hash {
        env.storage().instance().set(&DataKey::MetadataHash, &hash);
    }
    extend_ttl(&env);
    emit_init_events(&env, &buyer, &seller, &arbiter, total);
    Ok(())
}

/// Release a specific milestone's funds to the seller.
///
/// Can be called by the buyer or the arbiter while the escrow is `Funded`.
/// The milestone at `milestone_index` must not already have been released.
///
/// If a fee is configured, the fee amount is routed to the treasury and the
/// net amount goes to the seller.
///
/// When all milestones have been released the escrow state transitions to
/// `Completed`.
pub fn release_milestone(
    env: Env,
    caller: Address,
    milestone_index: u32,
) -> Result<(), EscrowError> {
    #[cfg(feature = "pausable")]
    crate::EscrowContract::require_not_paused(&env)?;

    let buyer: Address = get_required(&env, &Buyer)?;
    let arbiter: Address = get_required(&env, &Arbiter)?;

    // Only the buyer or the arbiter may release a milestone.
    if caller != buyer && caller != arbiter {
        return Err(EscrowError::NotAuthorized);
    }
    caller.require_auth();

    let state: EscrowState = get_required(&env, &State)?;
    if state != EscrowState::Funded {
        return Err(EscrowError::InvalidState);
    }

    let mut milestones: Vec<Milestone> = env
        .storage()
        .instance()
        .get(&DataKey::Milestones)
        .ok_or(EscrowError::MilestoneNotFound)?;

    let idx = milestone_index as usize;
    if idx >= milestones.len() as usize {
        return Err(EscrowError::MilestoneNotFound);
    }

    let mut m = milestones
        .get(milestone_index)
        .ok_or(EscrowError::MilestoneNotFound)?;
    if m.released {
        return Err(EscrowError::InvalidState);
    }
    let gross = m.amount;
    m.released = true;
    milestones.set(milestone_index, m);

    // Update stored milestones.
    env.storage()
        .instance()
        .set(&DataKey::Milestones, &milestones);

    // Deduct this milestone from the stored remaining amount so existing
    // queries still reflect unreleased funds.
    let remaining: i128 = get_required(&env, &Amount)?;
    #[allow(clippy::arithmetic_side_effects)]
    let new_remaining = remaining - gross;
    env.storage().instance().set(&Amount, &new_remaining);

    extend_ttl(&env);

    let seller: Address = get_required(&env, &Seller)?;
    let (net, fee) = apply_fee(&env, gross);
    admin::transfer_token(&env, &env.current_contract_address(), &seller, net);
    maybe_pay_fee(&env, fee);

    events::milestone_released(&env, &seller, milestone_index, net, fee);

    // Transition to Completed when all milestones are released.
    let all_done = milestones.iter().all(|m| m.released);
    if all_done {
        env.storage()
            .instance()
            .set(&State, &EscrowState::Completed);
        events::funds_released(&env, &seller, 0, 0);
    }

    Ok(())
}

/// Return the stored milestone list, or an empty Vec if none.
pub fn get_milestones(env: Env) -> Vec<Milestone> {
    env.storage()
        .instance()
        .get(&DataKey::Milestones)
        .unwrap_or_else(|| Vec::new(&env))
}
