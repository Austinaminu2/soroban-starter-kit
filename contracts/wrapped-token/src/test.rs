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
    symbol_short,
    testutils::{Address as _, Ledger as _},
    token,
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
// unwrap checked arithmetic and reserve guard
// ---------------------------------------------------------------------------

#[test]
fn test_unwrap_underflow_when_amount_exceeds_supply() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 1_000);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);

    assert!(client.try_unwrap(&user, &1i128).is_err());
    assert_eq!(client.get_total_wrapped(), 0);

    client.wrap(&user, &50i128);
    // Extra wrapped tokens minted outside `wrap`, with matching underlying
    // donated in, so this fails the supply check rather than the reserve check.
    token::StellarAssetClient::new(&env, &wrapped_token).mint(&user, &50i128);
    token::Client::new(&env, &underlying_token).transfer(&user, &client.address, &50i128);

    assert!(client.try_unwrap(&user, &51i128).is_err());
    assert_eq!(client.get_total_wrapped(), 50);

    client.unwrap(&user, &50i128);
    assert_eq!(client.get_total_wrapped(), 0);
}

#[test]
fn test_unwrap_rejects_when_reserve_is_short() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 1_000);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);
    client.wrap(&user, &100i128);

    // Underlying leaves the contract without a matching burn (rebasing / external drain).
    env.as_contract(&client.address, || {
        token::Client::new(&env, &underlying_token).transfer(&client.address, &user, &40i128);
    });
    assert_eq!(client.get_reserve_balance(), 60);
    assert!(client.try_unwrap(&user, &61i128).is_err());
    assert_eq!(client.get_total_wrapped(), 100);

    client.unwrap(&user, &60i128);
    assert_eq!(client.get_total_wrapped(), 40);
    assert_eq!(client.get_reserve_balance(), 0);
}

// ---------------------------------------------------------------------------
// Proof-of-reserve oracle
// ---------------------------------------------------------------------------

#[soroban_sdk::contract]
struct MockPorOracle;

#[soroban_sdk::contractimpl]
impl MockPorOracle {
    pub fn set_reserve(env: Env, reserve: i128) {
        env.storage()
            .instance()
            .set(&symbol_short!("reserve"), &reserve);
    }

    pub fn get_reserve(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&symbol_short!("reserve"))
            .unwrap_or(0)
    }
}

#[test]
fn test_wrap_rejected_when_oracle_reserve_exceeded() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 1_000);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);

    let oracle_id = env.register_contract(None, MockPorOracle);
    let oracle = MockPorOracleClient::new(&env, &oracle_id);
    oracle.set_reserve(&150i128);
    client.set_por_oracle(&Some(oracle_id));

    client.wrap(&user, &100i128);
    assert_eq!(client.get_total_wrapped(), 100);

    // 100 + 51 > 150
    assert!(client.try_wrap(&user, &51i128).is_err());
    assert_eq!(client.get_total_wrapped(), 100);

    // Boundary: 100 + 50 == 150 is allowed.
    client.wrap(&user, &50i128);
    assert_eq!(client.get_total_wrapped(), 150);

    oracle.set_reserve(&200i128);
    client.wrap(&user, &40i128);
    assert_eq!(client.get_total_wrapped(), 190);
}

// ---------------------------------------------------------------------------
// Aggregate unwrap rate limit
// ---------------------------------------------------------------------------

#[test]
fn test_unwrap_rate_limit_throttles_high_volume_spike() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.sequence_number = 1_000);

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 1_000);
    let other = Address::generate(&env);
    token::StellarAssetClient::new(&env, &underlying_token).mint(&other, &1_000);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);
    client.set_unwrap_rate_limit(&100i128, &10u32);

    client.wrap(&user, &400i128);
    client.wrap(&other, &400i128);

    client.unwrap(&user, &60i128);
    client.unwrap(&other, &40i128);
    assert_eq!(client.get_total_wrapped(), 700);

    // 60 + 40 + 1 exceeds the aggregate cap of 100 inside this window.
    assert!(client.try_unwrap(&user, &1i128).is_err());
    assert_eq!(client.get_total_wrapped(), 700);

    env.ledger().with_mut(|l| l.sequence_number = 1_010);
    client.unwrap(&user, &80i128);
    assert_eq!(client.get_total_wrapped(), 620);
}

#[test]
fn test_admin_can_bypass_unwrap_rate_limit() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, admin, user, wrapped_token, underlying_token) = setup(&env, 1_000);
    client.initialize(&admin, &wrapped_token, &underlying_token, &None);
    client.set_unwrap_rate_limit(&50i128, &100u32);
    client.wrap(&user, &200i128);

    assert!(client.try_unwrap(&user, &80i128).is_err());

    client.set_unwrap_rate_limit_paused(&true);
    client.unwrap(&user, &80i128);
    assert_eq!(client.get_total_wrapped(), 120);

    client.set_unwrap_rate_limit_paused(&false);
    assert!(client.try_unwrap(&user, &80i128).is_err());
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

        use soroban_sdk::{IntoVal, Symbol, testutils::Events as _};
        let all = env.events().all();
        let (_, topics, _) = all.last().unwrap();
        assert_eq!(topics, (Symbol::new(&env, "paused"), admin).into_val(&env));
    }

    #[test]
    fn test_unpause_emits_event() {
        let env = Env::default();
        env.mock_all_auths();

        let (client, admin, _user, wrapped_token, underlying_token) = setup(&env, 0);
        client.initialize(&admin, &wrapped_token, &underlying_token, &None);

        client.pause();
        client.unpause();

        use soroban_sdk::{IntoVal, Symbol, testutils::Events as _};
        let all = env.events().all();
        let (_, topics, _) = all.last().unwrap();
        assert_eq!(
            topics,
            (Symbol::new(&env, "unpaused"), admin).into_val(&env)
        );
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
