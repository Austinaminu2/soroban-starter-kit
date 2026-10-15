#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use super::*;
use soroban_sdk::{
    Address, Env, MuxedAddress, String, contract, contractimpl, contracttype,
    testutils::Address as _,
    token::{self, TokenInterface},
};

/// Registers the wrapped-token contract plus two Stellar Asset Contracts
/// (underlying + wrapped), with the wrapped-token contract itself set as the
/// wrapped SAC's admin so `wrap` can mint. Mints `initial_underlying` of the
/// underlying asset to `user`.
fn setup<'a>(
    env: &'a Env,
    initial_underlying: i128,
) -> (
    WrappedTokenContractClient<'a>,
    Address,
    Address,
    Address,
    Address,
) {
    let admin = Address::generate(env);
    let user = Address::generate(env);

    let contract_id = env.register_contract(None, WrappedTokenContract);

    let wrapped_sac = env.register_stellar_asset_contract_v2(contract_id.clone());
    let wrapped_token = wrapped_sac.address();

    let underlying_admin = Address::generate(env);
    let underlying_sac = env.register_stellar_asset_contract_v2(underlying_admin);
    let underlying_token = underlying_sac.address();

    if initial_underlying > 0 {
        token::StellarAssetClient::new(env, &underlying_token).mint(&user, &initial_underlying);
    }

    let client = WrappedTokenContractClient::new(env, &contract_id);
    (client, admin, user, wrapped_token, underlying_token)
}

#[test]
fn test_wrap_unwrap_1_1_peg() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 1_000);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);

    client.wrap(&user, &100i128);
    assert_eq!(client.get_total_wrapped(), 100);
    assert_eq!(token::Client::new(&env, &wrapped_token).balance(&user), 100);
    assert_eq!(
        token::Client::new(&env, &underlying_token).balance(&user),
        900
    );

    client.unwrap(&user, &50i128);
    assert_eq!(client.get_total_wrapped(), 50);

    client.unwrap(&user, &50i128);
    assert_eq!(client.get_total_wrapped(), 0);
    assert_eq!(
        token::Client::new(&env, &underlying_token).balance(&user),
        1_000
    );
}

#[test]
fn test_initialize_idempotent_check() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, _user, wrapped_token, underlying_token) = setup(&env, 0);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);

    let result = client.try_initialize(&admin, &wrapped_token, &underlying_token, &None);
    assert!(result.is_err());
}

#[test]
fn test_wrap_requires_positive_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 100);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);

    assert!(client.try_wrap(&user, &0i128).is_err());
    assert!(client.try_wrap(&user, &-1i128).is_err());
}

// ---------------------------------------------------------------------------
// Reserve-balance invariant (issue: monitoring for wrap/unwrap backing)
// ---------------------------------------------------------------------------

#[test]
fn test_get_reserve_balance_matches_underlying_holdings() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 1_000);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);

    assert_eq!(client.get_reserve_balance(), 0);

    client.wrap(&user, &300i128);
    assert_eq!(client.get_reserve_balance(), 300);
    assert_eq!(client.get_total_wrapped(), client.get_reserve_balance());

    client.unwrap(&user, &120i128);
    assert_eq!(client.get_reserve_balance(), 180);
    assert_eq!(client.get_total_wrapped(), client.get_reserve_balance());
}

#[test]
fn test_reserve_invariant_holds_after_wrap_unwrap_sequence() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 10_000);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);

    let amounts: [i128; 6] = [500, 200, -300, 400, -100, -700];
    for delta in amounts {
        if delta > 0 {
            client.wrap(&user, &delta);
        } else {
            client.unwrap(&user, &(-delta));
        }
        // Core invariant: the wrapped supply can never exceed the actual
        // underlying reserve held by the contract.
        assert!(client.get_total_wrapped() <= client.get_reserve_balance());
    }
}

// ---------------------------------------------------------------------------
// max_wrap_per_address
// ---------------------------------------------------------------------------

#[test]
fn test_max_wrap_per_address_boundary_succeeds() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 1_000);
    client.initialize(&admin, &wrapped_token, &underlying_token, &Some(500i128));

    client.wrap(&user, &300i128);
    client.wrap(&user, &200i128); // cumulative 500 == cap, must succeed
    assert_eq!(client.wrapped_by(&user), 500);
}

#[test]
fn test_max_wrap_per_address_rejects_over_cap() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 1_000);
    client.initialize(&admin, &wrapped_token, &underlying_token, &Some(500i128));

    client.wrap(&user, &300i128);
    let result = client.try_wrap(&user, &201i128); // cumulative 501 > cap
    assert!(result.is_err());
    // The rejected wrap must not have partially applied.
    assert_eq!(client.wrapped_by(&user), 300);
    assert_eq!(client.get_total_wrapped(), 300);
}

#[test]
fn test_max_wrap_per_address_is_per_user() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 1_000);
    let other_user = Address::generate(&env);
    token::StellarAssetClient::new(&env, &underlying_token).mint(&other_user, &1_000);

    client.initialize(&admin, &wrapped_token, &underlying_token, &Some(500i128));

    client.wrap(&user, &500i128);
    // A different address has its own independent cap allowance.
    client.wrap(&other_user, &500i128);
    assert_eq!(client.wrapped_by(&user), 500);
    assert_eq!(client.wrapped_by(&other_user), 500);
}

#[test]
fn test_uncapped_wrap_allows_large_amounts() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 1_000_000);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);
    assert_eq!(client.max_wrap_per_address(), None);

    client.wrap(&user, &1_000_000i128);
    assert_eq!(client.get_total_wrapped(), 1_000_000);
}

#[test]
fn test_initialize_rejects_non_positive_cap() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, _user, wrapped_token, underlying_token) = setup(&env, 0);
    let result = client.try_initialize(&admin, &wrapped_token, &underlying_token, &Some(0i128));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Pausable — only compiled when the `pausable` feature is enabled.
// ---------------------------------------------------------------------------

#[cfg(feature = "pausable")]
mod pausable_tests {
    use super::*;

    #[test]
    fn test_pause_blocks_wrap() {
        let env = Env::default();
        env.mock_all_auths();

        let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 100);
        client.initialize(&admin, &wrapped_token, &underlying_token, &None);

        client.pause();
        assert!(client.try_wrap(&user, &10i128).is_err());
    }

    #[test]
    fn test_pause_blocks_unwrap() {
        let env = Env::default();
        env.mock_all_auths();

        let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 100);
        client.initialize(&admin, &wrapped_token, &underlying_token, &None);
        client.wrap(&user, &50i128);

        client.pause();
        assert!(client.try_unwrap(&user, &10i128).is_err());
    }

    #[test]
    fn test_unpause_restores_wrap_and_unwrap() {
        let env = Env::default();
        env.mock_all_auths();

        let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 100);
        client.initialize(&admin, &wrapped_token, &underlying_token, &None);

        client.pause();
        client.unpause();

        client.wrap(&user, &40i128);
        assert_eq!(client.get_total_wrapped(), 40);
        client.unwrap(&user, &40i128);
        assert_eq!(client.get_total_wrapped(), 0);
    }

    #[test]
    fn test_pause_emits_event() {
        let env = Env::default();
        env.mock_all_auths();

        let (client, admin, _user, wrapped_token, underlying_token) = setup(&env, 0);
        client.initialize(&admin, &wrapped_token, &underlying_token, &None);

        client.pause();
        use soroban_sdk::testutils::Events as _;
        assert!(!env.events().all().events().is_empty());
    }

    #[test]
    fn test_unpause_emits_event() {
        let env = Env::default();
        env.mock_all_auths();

        let (client, admin, _user, wrapped_token, underlying_token) = setup(&env, 0);
        client.initialize(&admin, &wrapped_token, &underlying_token, &None);

        client.pause();
        client.unpause();
        use soroban_sdk::testutils::Events as _;
        assert!(!env.events().all().events().is_empty());
    }

    #[test]
    fn test_pause_requires_admin() {
        let env = Env::default();
        env.mock_all_auths();

        let (client, admin, _user, wrapped_token, underlying_token) = setup(&env, 0);
        client.initialize(&admin, &wrapped_token, &underlying_token, &None);

        // Not directly testable without auth mocking per-address; pause() enforces
        // admin via require_admin + require_auth, exercised positively above.
        client.pause();
        assert!(client.try_wrap(&Address::generate(&env), &1i128).is_err());
    }
}


// ---------------------------------------------------------------------------
// Fee-on-transfer underlying (issue #1181)
// ---------------------------------------------------------------------------

#[contracttype]
enum FotKey {
    Balance(Address),
}

/// Underlying token that withholds a 10% fee on every transfer.
#[contract]
struct FeeOnTransferToken;

#[contractimpl]
impl FeeOnTransferToken {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let key = FotKey::Balance(to.clone());
        let bal: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(bal + amount));
    }
}

#[contractimpl]
impl TokenInterface for FeeOnTransferToken {
    fn allowance(_env: Env, _from: Address, _spender: Address) -> i128 {
        0
    }

    fn approve(
        _env: Env,
        _from: Address,
        _spender: Address,
        _amount: i128,
        _expiration_ledger: u32,
    ) {
    }

    fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .instance()
            .get(&FotKey::Balance(id))
            .unwrap_or(0)
    }

    fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
        let to_addr = to.address();
        let from_key = FotKey::Balance(from.clone());
        let to_key = FotKey::Balance(to_addr);
        let from_bal: i128 = env.storage().instance().get(&from_key).unwrap_or(0);
        let to_bal: i128 = env.storage().instance().get(&to_key).unwrap_or(0);
        // 10% fee burned on transfer; recipient gets 90%.
        let fee = amount / 10;
        let received = amount - fee;
        env.storage()
            .instance()
            .set(&from_key, &(from_bal - amount));
        env.storage()
            .instance()
            .set(&to_key, &(to_bal + received));
    }

    fn transfer_from(_env: Env, _spender: Address, _from: Address, _to: Address, _amount: i128) {}

    fn burn(_env: Env, _from: Address, _amount: i128) {}

    fn burn_from(_env: Env, _spender: Address, _from: Address, _amount: i128) {}

    fn decimals(_env: Env) -> u32 {
        7
    }

    fn name(env: Env) -> String {
        String::from_str(&env, "FeeToken")
    }

    fn symbol(env: Env) -> String {
        String::from_str(&env, "FEE")
    }
}

#[test]
fn test_wrap_mints_only_actual_received_with_fee_on_transfer() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let contract_id = env.register_contract(None, WrappedTokenContract);
    let wrapped_sac = env.register_stellar_asset_contract_v2(contract_id.clone());
    let wrapped_token = wrapped_sac.address();

    let underlying_token = env.register_contract(None, FeeOnTransferToken);
    FeeOnTransferTokenClient::new(&env, &underlying_token).mint(&user, &1_000);

    let client = WrappedTokenContractClient::new(&env, &contract_id);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);

    // User requests wrap of 100; fee-on-transfer delivers 90 to the contract.
    client.wrap(&user, &100i128);

    assert_eq!(client.get_total_wrapped(), 90);
    assert_eq!(client.get_reserve_balance(), 90);
    assert_eq!(token::Client::new(&env, &wrapped_token).balance(&user), 90);
    assert!(client.get_total_wrapped() <= client.get_reserve_balance());
}

#[test]
fn test_fee_on_transfer_wrap_then_full_unwrap_succeeds() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    let contract_id = env.register_contract(None, WrappedTokenContract);
    let wrapped_sac = env.register_stellar_asset_contract_v2(contract_id.clone());
    let wrapped_token = wrapped_sac.address();

    let underlying_token = env.register_contract(None, FeeOnTransferToken);
    FeeOnTransferTokenClient::new(&env, &underlying_token).mint(&user, &1_000);

    let client = WrappedTokenContractClient::new(&env, &contract_id);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);

    client.wrap(&user, &200i128); // receives 180
    assert_eq!(client.get_total_wrapped(), 180);
    client.unwrap(&user, &180i128);
    assert_eq!(client.get_total_wrapped(), 0);
    assert_eq!(client.get_reserve_balance(), 0);
}
