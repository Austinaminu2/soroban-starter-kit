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
    Address, Env, Vec,
    testutils::{Address as _, Ledger as _},
    token::{Client as TokenClient, StellarAssetClient},
};

fn register_token(env: &Env) -> Address {
    let admin = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    sac.address()
}

fn mint(env: &Env, token: &Address, to: &Address, amount: i128) {
    StellarAssetClient::new(env, token).mint(to, &amount);
}

fn setup(env: &Env) -> (SwapContractClient, Address, Address, Address, Address, Address) {
    let token_a = register_token(env);
    let token_b = register_token(env);
    let party_a = Address::generate(env);
    let party_b = Address::generate(env);
    let treasury = Address::generate(env);

    mint(env, &token_a, &party_a, 100_000);
    mint(env, &token_b, &party_b, 100_000);

    let addr = env.register_contract(None, SwapContract);
    let client = SwapContractClient::new(env, &addr);
    client.initialize(&party_a, &treasury, &50); // 0.5% fee
    let approve_until = env.ledger().sequence() + 1_000_000;
    TokenClient::new(env, &token_a).approve(&party_a, &addr, &1_000_000_000, &approve_until);

    (client, party_a, party_b, token_a, token_b, treasury)
}

#[test]
fn test_propose_and_accept_full_swap() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, party_a, party_b, token_a, token_b, treasury) = setup(&env);

    let expires_at = env.ledger().sequence() + 100;
    let swap_id = client.propose_swap(&party_a, &token_a, &1_000, &token_b, &500, &expires_at);
    client.accept_swap(&swap_id, &party_b);

    let swap = client.get_swap(&swap_id);
    assert_eq!(swap.state, SwapState::Executed);
    assert_eq!(swap.filled_amount, 1_000);

    let token_a_client = TokenClient::new(&env, &token_a);
    let token_b_client = TokenClient::new(&env, &token_b);
    assert_eq!(token_a_client.balance(&party_b), 1_000);
    assert_eq!(token_b_client.balance(&treasury), 2); // 500 * 50 / 10000 = 2
}

#[test]
fn test_partial_fill_multiple_takes() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, party_a, party_b, token_a, token_b, _treasury) = setup(&env);

    let expires_at = env.ledger().sequence() + 100;
    let swap_id = client.propose_swap_with_options(
        &party_a, &token_a, &1_000, &token_b, &500, &expires_at, &true, &false,
    );

    client.accept_swap_partial(&party_b, &swap_id, &400);
    let swap = client.get_swap(&swap_id);
    assert_eq!(swap.state, SwapState::Pending);
    assert_eq!(swap.filled_amount, 400);

    client.accept_swap_partial(&party_b, &swap_id, &600);
    let swap = client.get_swap(&swap_id);
    assert_eq!(swap.state, SwapState::Executed);
    assert_eq!(swap.filled_amount, 1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_partial_fill_rejected_when_not_enabled() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, party_a, party_b, token_a, token_b, _treasury) = setup(&env);

    let expires_at = env.ledger().sequence() + 100;
    let swap_id = client.propose_swap(&party_a, &token_a, &1_000, &token_b, &500, &expires_at);
    client.accept_swap_partial(&party_b, &swap_id, &500);
}

#[test]
fn test_escrowed_cancel_returns_unfilled_tokens() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, party_a, party_b, token_a, token_b, _treasury) = setup(&env);
    let token_a_client = TokenClient::new(&env, &token_a);
    let contract_addr = client.address.clone();

    let balance_before = token_a_client.balance(&party_a);
    let expires_at = env.ledger().sequence() + 100;
    let swap_id = client.propose_swap_with_options(
        &party_a, &token_a, &1_000, &token_b, &500, &expires_at, &true, &true,
    );
    assert_eq!(token_a_client.balance(&contract_addr), 1_000);
    assert_eq!(token_a_client.balance(&party_a), balance_before - 1_000);

    client.accept_swap_partial(&party_b, &swap_id, &400);
    assert_eq!(token_a_client.balance(&contract_addr), 600);

    client.cancel_swap(&swap_id);
    assert_eq!(token_a_client.balance(&contract_addr), 0);
    assert_eq!(token_a_client.balance(&party_a), balance_before - 400);
    assert_eq!(client.get_swap(&swap_id).state, SwapState::Cancelled);
}

#[test]
fn test_escrowed_accept_transfers_from_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, party_a, party_b, token_a, token_b, _treasury) = setup(&env);
    let token_a_client = TokenClient::new(&env, &token_a);
    let contract_addr = client.address.clone();

    let expires_at = env.ledger().sequence() + 100;
    let swap_id = client.propose_swap_with_options(
        &party_a, &token_a, &900, &token_b, &450, &expires_at, &false, &true,
    );
    client.accept_swap(&swap_id, &party_b);

    assert_eq!(token_a_client.balance(&contract_addr), 0);
    assert_eq!(token_a_client.balance(&party_b), 900);
    assert_eq!(client.get_swap(&swap_id).state, SwapState::Executed);
}

#[test]
fn test_basket_swap_success() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, party_a, party_b, token_a, token_b, _treasury) = setup(&env);
    let token_c = register_token(&env);
    let token_d = register_token(&env);

    mint(&env, &token_c, &party_a, 2_000);
    mint(&env, &token_d, &party_b, 2_000);
    let approve_until = env.ledger().sequence() + 1_000_000;
    TokenClient::new(&env, &token_c).approve(&party_a, &client.address, &1_000_000_000, &approve_until);

    let offers = Vec::from_array(
        &env,
        [
            BasketLeg {
                token: token_a.clone(),
                amount: 500,
            },
            BasketLeg {
                token: token_c.clone(),
                amount: 700,
            },
        ],
    );
    let demands = Vec::from_array(
        &env,
        [
            BasketLeg {
                token: token_b.clone(),
                amount: 300,
            },
            BasketLeg {
                token: token_d.clone(),
                amount: 400,
            },
        ],
    );
    let expires_at = env.ledger().sequence() + 100;
    let basket_id = client.propose_basket_swap(&party_a, &offers, &demands, &expires_at);
    client.accept_basket_swap(&basket_id, &party_b);

    let info = client.get_basket_swap(&basket_id);
    assert_eq!(info.state, SwapState::Executed);
    assert_eq!(TokenClient::new(&env, &token_c).balance(&party_b), 700);
    assert_eq!(TokenClient::new(&env, &token_d).balance(&party_a), 400);
}

#[test]
fn test_basket_swap_atomic_failure_rolls_back() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, party_a, party_b, token_a, token_b, _treasury) = setup(&env);

    let offers = Vec::from_array(
        &env,
        [BasketLeg {
            token: token_a.clone(),
            amount: 500,
        }],
    );
    let demands = Vec::from_array(
        &env,
        [BasketLeg {
            token: token_b.clone(),
            amount: 200_000, // impossible for party_b
        }],
    );
    let expires_at = env.ledger().sequence() + 100;
    let basket_id = client.propose_basket_swap(&party_a, &offers, &demands, &expires_at);

    let party_a_before = TokenClient::new(&env, &token_b).balance(&party_a);
    let party_b_before = TokenClient::new(&env, &token_a).balance(&party_b);
    let result = client.try_accept_basket_swap(&basket_id, &party_b);
    assert!(result.is_err());

    // No partial transfers should persist.
    assert_eq!(TokenClient::new(&env, &token_b).balance(&party_a), party_a_before);
    assert_eq!(TokenClient::new(&env, &token_a).balance(&party_b), party_b_before);
    assert_eq!(client.get_basket_swap(&basket_id).state, SwapState::Pending);
}

#[test]
fn test_cancel_after_expiry_without_party_a_auth() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, party_a, _party_b, token_a, token_b, _treasury) = setup(&env);
    let expires_at = env.ledger().sequence() + 5;
    let swap_id = client.propose_swap(&party_a, &token_a, &1000, &token_b, &500, &expires_at);
    env.ledger().with_mut(|l| l.sequence_number = expires_at + 1);
    client.cancel_swap(&swap_id);
    assert_eq!(client.get_swap(&swap_id).state, SwapState::Cancelled);
}
