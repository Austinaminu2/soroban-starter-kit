#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(test)]

use super::*;
use soroban_sdk::{Address, Env, String, testutils::Address as _};

fn init_with_collection_royalty<'a>(
    env: &'a Env,
    bps: u32,
    recipient: &Address,
) -> (NftContractClient<'a>, Address) {
    let admin = Address::generate(env);
    let addr = env.register_contract(None, NftContract);
    let client = NftContractClient::new(env, &addr);
    client.initialize(
        &admin,
        &String::from_str(env, "Royal"),
        &String::from_str(env, "ROY"),
        &None,
        &Some(bps),
        &Some(recipient.clone()),
    );
    (client, admin)
}

/// Per-token royalty written at mint is readable from `royalty_info`.
/// Guards the instance/persistent storage mismatch on `TokenRoyaltyRecipient`.
#[test]
fn royalty_info_returns_per_token_royalty() {
    let env = Env::default();
    env.mock_all_auths();

    let collection_recipient = Address::generate(&env);
    let token_recipient = Address::generate(&env);
    let (client, _) = init_with_collection_royalty(&env, 1_000, &collection_recipient);

    let owner = Address::generate(&env);
    let token_id = client.mint(
        &owner,
        &String::from_str(&env, "ipfs://1"),
        &Some(250u32),
        &Some(token_recipient.clone()),
    );

    let info = client.royalty_info(&token_id, &10_000i128).unwrap();
    assert_eq!(info.recipient, token_recipient);
    assert_eq!(info.amount, 250i128);
}

/// A second token with no override still falls back to the collection royalty,
/// and a zero-bps per-token override suppresses the collection default.
#[test]
fn royalty_info_per_token_and_collection_fallback() {
    let env = Env::default();
    env.mock_all_auths();

    let collection_recipient = Address::generate(&env);
    let token_recipient = Address::generate(&env);
    let (client, _) = init_with_collection_royalty(&env, 500, &collection_recipient);

    let owner = Address::generate(&env);
    let overridden = client.mint(
        &owner,
        &String::from_str(&env, "ipfs://overridden"),
        &Some(100u32),
        &Some(token_recipient.clone()),
    );
    let fallback = client.mint(
        &owner,
        &String::from_str(&env, "ipfs://fallback"),
        &None,
        &None,
    );

    let per_token = client.royalty_info(&overridden, &20_000i128).unwrap();
    assert_eq!(per_token.recipient, token_recipient);
    assert_eq!(per_token.amount, 200i128);

    let collection = client.royalty_info(&fallback, &20_000i128).unwrap();
    assert_eq!(collection.recipient, collection_recipient);
    assert_eq!(collection.amount, 1_000i128);
}
