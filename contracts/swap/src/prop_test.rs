#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::unwrap_used
)]

extern crate std;

use super::*;
use proptest::prelude::*;
use soroban_sdk::{
    Address, Env,
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
};
use std::vec::Vec;

#[derive(Clone, Debug)]
enum Command {
    Propose { amount_a: i128, amount_b: i128 },
    Accept { index: usize },
    Cancel { index: usize },
}

fn commands() -> impl Strategy<Value = Vec<Command>> {
    prop::collection::vec(
        prop_oneof![
            (1i128..=1_000, 1i128..=1_000)
                .prop_map(|(amount_a, amount_b)| Command::Propose { amount_a, amount_b }),
            (0usize..=31).prop_map(|index| Command::Accept { index }),
            (0usize..=31).prop_map(|index| Command::Cancel { index }),
        ],
        1..=32,
    )
}

fn setup(
    env: &Env,
) -> (
    SwapContractClient,
    Address,
    Address,
    Address,
    Address,
    Address,
) {
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| ledger.sequence_number = 1);
    let admin = Address::generate(env);
    let treasury = Address::generate(env);
    let party_a = Address::generate(env);
    let party_b = Address::generate(env);
    let token_a = env
        .register_stellar_asset_contract_v2(Address::generate(env))
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(Address::generate(env))
        .address();
    StellarAssetClient::new(env, &token_a).mint(&party_a, &1_000_000i128);
    StellarAssetClient::new(env, &token_b).mint(&party_b, &1_000_000i128);
    let address = env.register_contract(None, SwapContract);
    let client = SwapContractClient::new(env, &address);
    client.initialize(&admin, &treasury, &250);
    (client, party_a, party_b, treasury, token_a, token_b)
}

fn assert_conservation(
    env: &Env,
    client: &SwapContractClient,
    party_a: &Address,
    party_b: &Address,
    treasury: &Address,
    token_a: &Address,
    token_b: &Address,
) {
    let contract = client.address.clone();
    let token_a_client = soroban_sdk::token::Client::new(env, token_a);
    let token_b_client = soroban_sdk::token::Client::new(env, token_b);
    let token_a_total = token_a_client.balance(party_a)
        + token_a_client.balance(party_b)
        + token_a_client.balance(&contract);
    let token_b_total = token_b_client.balance(party_a)
        + token_b_client.balance(party_b)
        + token_b_client.balance(treasury)
        + token_b_client.balance(&contract);
    assert_eq!(token_a_total, 1_000_000, "token A was created or destroyed");
    assert_eq!(token_b_total, 1_000_000, "token B was created or destroyed");
}

proptest! {
    /// Stateful invariant: arbitrary propose/cancel/accept sequences conserve both
    /// assets exactly, with fees moving only to the configured treasury.
    ///
    /// Closes #1082.
    #[test]
    fn prop_swap_state_machine_preserves_balances(actions in commands()) {
        let env = Env::default();
        let (client, party_a, party_b, treasury, token_a, token_b) = setup(&env);
        assert_conservation(&env, &client, &party_a, &party_b, &treasury, &token_a, &token_b);

        for action in actions {
            match action {
                Command::Propose { amount_a, amount_b } => {
                    let expiry = env.ledger().sequence() + 100;
                    let _ = client.try_propose_swap(
                        &party_a, &token_a, &amount_a, &token_b, &amount_b, &expiry,
                    );
                }
                Command::Accept { index } => {
                    let count = client.swap_count();
                    if count > 0 {
                        let id = (index as u32) % count;
                        let _ = client.try_accept_swap(&id, &party_b);
                    }
                }
                Command::Cancel { index } => {
                    let count = client.swap_count();
                    if count > 0 {
                        let id = (index as u32) % count;
                        let _ = client.try_cancel_swap(&id);
                    }
                }
            }
            assert_conservation(&env, &client, &party_a, &party_b, &treasury, &token_a, &token_b);
        }
    }
}
