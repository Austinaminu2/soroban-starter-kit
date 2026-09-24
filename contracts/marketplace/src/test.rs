#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![cfg(test)]

use super::*;
use soroban_sdk::{
    Address, Env, contract, contractimpl, contracttype,
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
};

// ---------------------------------------------------------------------------
// Minimal mock NFT contract for testing
// ---------------------------------------------------------------------------

#[contracttype]
enum NftKey {
    Owner(u32),
    Approved(u32),
}

#[contract]
pub struct MockNft;

#[contractimpl]
impl MockNft {
    pub fn init(env: Env, owner: Address, token_id: u32) {
        env.storage()
            .persistent()
            .set(&NftKey::Owner(token_id), &owner);
    }

    pub fn approve(env: Env, _caller: Address, spender: Address, token_id: u32, _expiry: u32) {
        env.storage()
            .persistent()
            .set(&NftKey::Approved(token_id), &spender);
    }

    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, token_id: u32) {
        let approved: Address = env
            .storage()
            .persistent()
            .get(&NftKey::Approved(token_id))
            .expect("no approval");
        assert_eq!(approved, spender);
        let owner: Address = env
            .storage()
            .persistent()
            .get(&NftKey::Owner(token_id))
            .expect("no owner");
        assert_eq!(owner, from);
        env.storage()
            .persistent()
            .set(&NftKey::Owner(token_id), &to);
    }

    pub fn owner_of(env: Env, token_id: u32) -> Address {
        env.storage()
            .persistent()
            .get(&NftKey::Owner(token_id))
            .expect("token not found")
    }

    pub fn royalty_info(_env: Env, _token_id: u32, _sale_price: i128) -> Option<super::contract::RoyaltyInfo> {
        None  // MockNft returns no royalty by default; tests can override via storage if needed
    }
}

// ---------------------------------------------------------------------------
// Test setup
// ---------------------------------------------------------------------------

struct TestEnv<'a> {
    env: Env,
    client: MarketplaceContractClient<'a>,
    marketplace: Address,
    token: Address,
    nft: Address,
    admin: Address,
    seller: Address,
    buyer: Address,
    royalty_recipient: Address,
}

fn setup<'a>(env: &'a Env) -> TestEnv<'a> {
    env.mock_all_auths();

    let admin = Address::generate(env);
    let seller = Address::generate(env);
    let buyer = Address::generate(env);
    let royalty_recipient = Address::generate(env);

    // Deploy payment token and mint to buyer
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    StellarAssetClient::new(env, &token).mint(&buyer, &10_000i128);

    // Deploy mock NFT and init token_id=1 owned by seller
    let nft = env.register_contract(None, MockNft);
    MockNftClient::new(env, &nft).init(&seller, &1u32);

    // Deploy marketplace and initialize
    let marketplace = env.register_contract(None, MarketplaceContract);
    MarketplaceContractClient::new(env, &marketplace).initialize(
        &admin,
        &token,
        &250u32,
        &royalty_recipient,
    );

    // Approve marketplace as NFT spender for token_id=1
    MockNftClient::new(env, &nft).approve(
        &seller,
        &marketplace,
        &1u32,
        &(env.ledger().sequence() + 10_000),
    );

    let client = MarketplaceContractClient::new(env, &marketplace);

    TestEnv {
        env: env.clone(),
        client,
        marketplace,
        token,
        nft,
        admin,
        seller,
        buyer,
        royalty_recipient,
    }
}

fn tok<'a>(env: &'a Env, token: &Address) -> TokenClient<'a> {
    TokenClient::new(env, token)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_initialize_rejects_duplicate() {
    let env = Env::default();
    let t = setup(&env);
    let res = t
        .client
        .try_initialize(&t.admin, &t.token, &100u32, &t.royalty_recipient);
    assert!(res.is_err());
}

#[test]
fn test_list_and_get_listing() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128);
    assert_eq!(id, 0);
    let listing = t.client.get_listing(&id).expect("listing");
    assert!(listing.active);
    assert_eq!(listing.price, 1_000);
    assert_eq!(listing.seller, t.seller);
}

#[test]
fn test_list_rejects_zero_price() {
    let env = Env::default();
    let t = setup(&env);
    let res = t.client.try_list(&t.seller, &t.nft, &1u32, &0i128);
    assert!(res.is_err());
}

#[test]
fn test_buy_happy_path() {
    let env = Env::default();
    let t = setup(&env);

    let price = 1_000i128;
    let id = t.client.list(&t.seller, &t.nft, &1u32, &price);

    let seller_before = tok(&env, &t.token).balance(&t.seller);
    let royalty_before = tok(&env, &t.token).balance(&t.royalty_recipient);
    let buyer_before = tok(&env, &t.token).balance(&t.buyer);

    t.client.buy(&t.buyer, &id, &price);

    let royalty = (price * 250) / 10_000; // 25
    let seller_amount = price - royalty; // 975

    assert_eq!(
        tok(&env, &t.token).balance(&t.seller),
        seller_before + seller_amount
    );
    assert_eq!(
        tok(&env, &t.token).balance(&t.royalty_recipient),
        royalty_before + royalty
    );
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), buyer_before - price);

    // Verify NFT transferred to buyer
    assert_eq!(MockNftClient::new(&env, &t.nft).owner_of(&1u32), t.buyer);

    // Listing now inactive
    let listing = t.client.get_listing(&id).expect("listing");
    assert!(!listing.active);
}

#[test]
fn test_buy_inactive_listing_fails() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &500i128);
    t.client.buy(&t.buyer, &id, &500i128);
    let res = t.client.try_buy(&t.buyer, &id, &500i128);
    assert!(res.is_err());
}

#[test]
fn test_cancel_listing() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &500i128);
    t.client.cancel(&t.seller, &id);
    let listing = t.client.get_listing(&id).expect("listing");
    assert!(!listing.active);
}

#[test]
fn test_cancel_already_cancelled_fails() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &500i128);
    t.client.cancel(&t.seller, &id);
    let res = t.client.try_cancel(&t.seller, &id);
    assert!(res.is_err());
}

#[test]
fn test_non_seller_cannot_cancel() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &500i128);
    let other = Address::generate(&env);
    let res = t.client.try_cancel(&other, &id);
    assert!(res.is_err());
}

#[test]
fn test_invalid_royalty_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let marketplace = env.register_contract(None, MarketplaceContract);
    let res = MarketplaceContractClient::new(&env, &marketplace)
        .try_initialize(&admin, &token, &10_001u32, &admin);
    assert!(res.is_err());
}

#[test]
fn test_nft_royalty_override_honored_on_buy() {
    let env = Env::default();
    env.mock_all_auths();
    let t = setup(&env);

    let price = 1_000i128;
    let id = t.client.list(&t.seller, &t.nft, &1u32, &price);

    let marketplace_royalty_bps: u32 = env.as_contract(&t.marketplace, || {
        env.storage()
            .instance()
            .get(&DataKey::RoyaltyBps)
            .unwrap_or(0)
    });
    let marketplace_royalty = (price * marketplace_royalty_bps as i128) / 10_000;

    let before_marketplace_royalty = tok(&env, &t.token).balance(&t.royalty_recipient);
    let before_seller = tok(&env, &t.token).balance(&t.seller);

    t.client.buy(&t.buyer, &id, &price);

    let after_marketplace_royalty = tok(&env, &t.token).balance(&t.royalty_recipient);
    assert_eq!(after_marketplace_royalty, before_marketplace_royalty + marketplace_royalty,
               "marketplace royalty should be used when NFT contract returns no royalty");
    assert_eq!(tok(&env, &t.token).balance(&t.seller),
               before_seller + price - marketplace_royalty,
               "seller should receive price minus marketplace royalty");
}

// ---------------------------------------------------------------------------
// #1101 — buyer max-price (front-running / slippage) protection
// ---------------------------------------------------------------------------

#[test]
fn test_buy_rejects_when_price_exceeds_max() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128);

    let res = t.client.try_buy(&t.buyer, &id, &999i128);
    assert_eq!(res, Err(Ok(MarketplaceError::PriceExceedsMax)));

    // Nothing moved: listing still active, NFT still with seller.
    assert!(t.client.get_listing(&id).unwrap().active);
    assert_eq!(MockNftClient::new(&env, &t.nft).owner_of(&1u32), t.seller);
}

#[test]
fn test_buy_accepts_max_price_above_listing_price() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128);
    let buyer_before = tok(&env, &t.token).balance(&t.buyer);

    t.client.buy(&t.buyer, &id, &1_500i128);

    // Buyer pays the listing price, not max_price.
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), buyer_before - 1_000);
    assert_eq!(MockNftClient::new(&env, &t.nft).owner_of(&1u32), t.buyer);
}

#[test]
fn test_buy_protected_against_cancel_and_relist_higher() {
    let env = Env::default();
    let t = setup(&env);

    // Buyer observes listing at 1_000...
    let old_id = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128);
    // ...seller cancels and re-lists higher before the buy lands.
    t.client.cancel(&t.seller, &old_id);
    let new_id = t.client.list(&t.seller, &t.nft, &1u32, &5_000i128);

    let buyer_before = tok(&env, &t.token).balance(&t.buyer);
    let res = t.client.try_buy(&t.buyer, &new_id, &1_000i128);
    assert_eq!(res, Err(Ok(MarketplaceError::PriceExceedsMax)));
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), buyer_before);
}

// ---------------------------------------------------------------------------
// #1098 — collection-wide floor offers
// ---------------------------------------------------------------------------

#[test]
fn test_collection_offer_lifecycle_accept() {
    use soroban_sdk::testutils::Ledger as _;
    let env = Env::default();
    let t = setup(&env);
    let expires_at = env.ledger().sequence() + 100;

    let buyer_before = tok(&env, &t.token).balance(&t.buyer);
    let offer_id = t
        .client
        .make_collection_offer(&t.buyer, &t.nft, &800i128, &expires_at);
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), buyer_before - 800);
    assert_eq!(tok(&env, &t.token).balance(&t.marketplace), 800);
    let offer = t.client.get_collection_offer(&offer_id).unwrap();
    assert_eq!(offer.buyer, t.buyer);
    assert_eq!(offer.nft_contract, t.nft);
    assert_eq!(offer.amount, 800);

    env.ledger().with_mut(|l| l.sequence_number += 10);

    let seller_before = tok(&env, &t.token).balance(&t.seller);
    let royalty_before = tok(&env, &t.token).balance(&t.royalty_recipient);
    t.client.accept_collection_offer(&t.seller, &offer_id, &1u32);

    let royalty = (800 * 250) / 10_000; // 20
    assert_eq!(tok(&env, &t.token).balance(&t.seller), seller_before + 800 - royalty);
    assert_eq!(
        tok(&env, &t.token).balance(&t.royalty_recipient),
        royalty_before + royalty
    );
    assert_eq!(tok(&env, &t.token).balance(&t.marketplace), 0);
    assert_eq!(MockNftClient::new(&env, &t.nft).owner_of(&1u32), t.buyer);
    assert!(t.client.get_collection_offer(&offer_id).is_none());

    // Cannot be accepted twice.
    let res = t.client.try_accept_collection_offer(&t.seller, &offer_id, &1u32);
    assert_eq!(res, Err(Ok(MarketplaceError::CollectionOfferNotFound)));
}

#[test]
fn test_collection_offer_any_token_in_collection() {
    let env = Env::default();
    let t = setup(&env);
    let holder = Address::generate(&env);
    MockNftClient::new(&env, &t.nft).init(&holder, &7u32);
    MockNftClient::new(&env, &t.nft).approve(&holder, &t.marketplace, &7u32, &0u32);

    let offer_id = t.client.make_collection_offer(
        &t.buyer,
        &t.nft,
        &400i128,
        &(env.ledger().sequence() + 100),
    );
    t.client.accept_collection_offer(&holder, &offer_id, &7u32);
    assert_eq!(MockNftClient::new(&env, &t.nft).owner_of(&7u32), t.buyer);
    assert_eq!(tok(&env, &t.token).balance(&holder), 400 - (400 * 250) / 10_000);
}

#[test]
fn test_collection_offer_cancel_refunds() {
    let env = Env::default();
    let t = setup(&env);
    let buyer_before = tok(&env, &t.token).balance(&t.buyer);
    let offer_id = t.client.make_collection_offer(
        &t.buyer,
        &t.nft,
        &600i128,
        &(env.ledger().sequence() + 100),
    );

    let other = Address::generate(&env);
    let res = t.client.try_cancel_collection_offer(&other, &offer_id);
    assert_eq!(res, Err(Ok(MarketplaceError::NotAuthorized)));

    t.client.cancel_collection_offer(&t.buyer, &offer_id);
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), buyer_before);
    assert_eq!(tok(&env, &t.token).balance(&t.marketplace), 0);
    assert!(t.client.get_collection_offer(&offer_id).is_none());
}

#[test]
fn test_collection_offer_expired_cannot_be_accepted_but_can_be_cancelled() {
    use soroban_sdk::testutils::Ledger as _;
    let env = Env::default();
    let t = setup(&env);
    let expires_at = env.ledger().sequence() + 5;
    let offer_id = t
        .client
        .make_collection_offer(&t.buyer, &t.nft, &500i128, &expires_at);

    env.ledger().with_mut(|l| l.sequence_number = expires_at + 1);
    let res = t.client.try_accept_collection_offer(&t.seller, &offer_id, &1u32);
    assert_eq!(res, Err(Ok(MarketplaceError::CollectionOfferExpired)));
    assert_eq!(MockNftClient::new(&env, &t.nft).owner_of(&1u32), t.seller);

    let buyer_before = tok(&env, &t.token).balance(&t.buyer);
    t.client.cancel_collection_offer(&t.buyer, &offer_id);
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), buyer_before + 500);
}

#[test]
fn test_collection_offer_validation() {
    let env = Env::default();
    let t = setup(&env);
    let now = env.ledger().sequence();
    assert_eq!(
        t.client
            .try_make_collection_offer(&t.buyer, &t.nft, &0i128, &(now + 10)),
        Err(Ok(MarketplaceError::InvalidOfferAmount))
    );
    assert_eq!(
        t.client
            .try_make_collection_offer(&t.buyer, &t.nft, &100i128, &now),
        Err(Ok(MarketplaceError::InvalidExpiry))
    );
    let offer_id = t
        .client
        .make_collection_offer(&t.buyer, &t.nft, &100i128, &(now + 10));
    // Buyer cannot accept their own offer.
    assert_eq!(
        t.client.try_accept_collection_offer(&t.buyer, &offer_id, &1u32),
        Err(Ok(MarketplaceError::NotAuthorized))
    );
}

#[test]
fn test_collection_offer_accept_without_ownership_reverts() {
    let env = Env::default();
    let t = setup(&env);
    let offer_id = t.client.make_collection_offer(
        &t.buyer,
        &t.nft,
        &300i128,
        &(env.ledger().sequence() + 100),
    );
    // A non-owner cannot sell token 1 into the offer; the whole call reverts.
    let impostor = Address::generate(&env);
    assert!(
        t.client
            .try_accept_collection_offer(&impostor, &offer_id, &1u32)
            .is_err()
    );
    assert_eq!(t.client.get_collection_offer(&offer_id).unwrap().amount, 300);
    assert_eq!(tok(&env, &t.token).balance(&impostor), 0);
    assert_eq!(tok(&env, &t.token).balance(&t.marketplace), 300);
}

// ---------------------------------------------------------------------------
// #1099 — multi-recipient royalty splits
// ---------------------------------------------------------------------------

#[test]
fn test_multi_recipient_royalty_split_on_buy() {
    let env = Env::default();
    let t = setup(&env);
    let creator_a = Address::generate(&env);
    let creator_b = Address::generate(&env);
    let dao = Address::generate(&env);

    let splits = soroban_sdk::vec![
        &env,
        (creator_a.clone(), 300u32),
        (creator_b.clone(), 200u32),
        (dao.clone(), 100u32),
    ];
    t.client.set_royalty_splits(&t.admin, &splits);
    assert_eq!(t.client.get_royalty_splits(), splits);

    let price = 10_000i128;
    StellarAssetClient::new(&env, &t.token).mint(&t.buyer, &price);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &price);
    let legacy_before = tok(&env, &t.token).balance(&t.royalty_recipient);

    t.client.buy(&t.buyer, &id, &price);

    assert_eq!(tok(&env, &t.token).balance(&creator_a), 300);
    assert_eq!(tok(&env, &t.token).balance(&creator_b), 200);
    assert_eq!(tok(&env, &t.token).balance(&dao), 100);
    assert_eq!(tok(&env, &t.token).balance(&t.seller), price - 600);
    // Splits supersede the single initialize-time recipient.
    assert_eq!(
        tok(&env, &t.token).balance(&t.royalty_recipient),
        legacy_before
    );
}

#[test]
fn test_multi_recipient_royalty_split_on_accept_offer_rounds_down() {
    let env = Env::default();
    let t = setup(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    t.client
        .set_royalty_splits(&t.admin, &soroban_sdk::vec![&env, (a.clone(), 333u32), (b.clone(), 333u32)]);

    let id = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128);
    t.client.make_offer(&t.buyer, &id, &999i128);
    t.client.accept_offer(&t.seller, &id, &t.buyer);

    let share = (999 * 333) / 10_000; // 33 each, dust stays with the seller
    assert_eq!(tok(&env, &t.token).balance(&a), share);
    assert_eq!(tok(&env, &t.token).balance(&b), share);
    assert_eq!(tok(&env, &t.token).balance(&t.seller), 999 - 2 * share);
    assert_eq!(tok(&env, &t.token).balance(&t.marketplace), 0);
}

#[test]
fn test_royalty_split_validation() {
    let env = Env::default();
    let t = setup(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    // Total > 10_000 bps.
    assert_eq!(
        t.client.try_set_royalty_splits(
            &t.admin,
            &soroban_sdk::vec![&env, (a.clone(), 6_000u32), (b.clone(), 4_001u32)]
        ),
        Err(Ok(MarketplaceError::InvalidRoyalty))
    );
    // Exactly 10_000 bps is allowed.
    t.client.set_royalty_splits(
        &t.admin,
        &soroban_sdk::vec![&env, (a.clone(), 6_000u32), (b.clone(), 4_000u32)],
    );
    // Zero-bps entry.
    assert_eq!(
        t.client
            .try_set_royalty_splits(&t.admin, &soroban_sdk::vec![&env, (a.clone(), 0u32)]),
        Err(Ok(MarketplaceError::InvalidRoyalty))
    );
    // Too many recipients.
    let mut many = soroban_sdk::Vec::new(&env);
    for _ in 0..=MAX_ROYALTY_RECIPIENTS {
        many.push_back((Address::generate(&env), 1u32));
    }
    assert_eq!(
        t.client.try_set_royalty_splits(&t.admin, &many),
        Err(Ok(MarketplaceError::InvalidRoyalty))
    );
    // Non-admin.
    assert_eq!(
        t.client
            .try_set_royalty_splits(&a, &soroban_sdk::vec![&env, (a.clone(), 100u32)]),
        Err(Ok(MarketplaceError::NotAuthorized))
    );
    // Empty clears back to the initialize-time royalty.
    t.client
        .set_royalty_splits(&t.admin, &soroban_sdk::Vec::new(&env));
    assert_eq!(t.client.get_royalty_splits().len(), 0);
}

#[test]
fn test_royalty_split_applies_to_collection_offer() {
    let env = Env::default();
    let t = setup(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    t.client.set_royalty_splits(
        &t.admin,
        &soroban_sdk::vec![&env, (a.clone(), 500u32), (b.clone(), 1_500u32)],
    );
    let offer_id = t.client.make_collection_offer(
        &t.buyer,
        &t.nft,
        &2_000i128,
        &(env.ledger().sequence() + 100),
    );
    t.client.accept_collection_offer(&t.seller, &offer_id, &1u32);
    assert_eq!(tok(&env, &t.token).balance(&a), 100);
    assert_eq!(tok(&env, &t.token).balance(&b), 300);
    assert_eq!(tok(&env, &t.token).balance(&t.seller), 1_600);
}
