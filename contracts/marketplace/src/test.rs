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

    /// Test helper: move ownership without going through the marketplace,
    /// simulating a seller transferring the NFT away after listing it.
    pub fn set_owner(env: Env, token_id: u32, owner: Address) {
        env.storage()
            .persistent()
            .set(&NftKey::Owner(token_id), &owner);
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
    let id = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128, &t.token);
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
    let res = t.client.try_list(&t.seller, &t.nft, &1u32, &0i128, &t.token);
    assert!(res.is_err());
}

#[test]
fn test_buy_happy_path() {
    let env = Env::default();
    let t = setup(&env);

    let price = 1_000i128;
    let id = t.client.list(&t.seller, &t.nft, &1u32, &price, &t.token);

    let seller_before = tok(&env, &t.token).balance(&t.seller);
    let royalty_before = tok(&env, &t.token).balance(&t.royalty_recipient);
    let buyer_before = tok(&env, &t.token).balance(&t.buyer);

    t.client.buy(&t.buyer, &id);

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
    let id = t.client.list(&t.seller, &t.nft, &1u32, &500i128, &t.token);
    t.client.buy(&t.buyer, &id);
    let res = t.client.try_buy(&t.buyer, &id);
    assert!(res.is_err());
}

#[test]
fn test_cancel_listing() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &500i128, &t.token);
    t.client.cancel(&t.seller, &id);
    let listing = t.client.get_listing(&id).expect("listing");
    assert!(!listing.active);
}

#[test]
fn test_cancel_already_cancelled_fails() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &500i128, &t.token);
    t.client.cancel(&t.seller, &id);
    let res = t.client.try_cancel(&t.seller, &id);
    assert!(res.is_err());
}

#[test]
fn test_non_seller_cannot_cancel() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &500i128, &t.token);
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
    let id = t.client.list(&t.seller, &t.nft, &1u32, &price, &t.token);

    let marketplace_royalty_bps: u32 = env.as_contract(&t.marketplace, || {
        env.storage()
            .instance()
            .get(&DataKey::RoyaltyBps)
            .unwrap_or(0)
    });
    let marketplace_royalty = (price * marketplace_royalty_bps as i128) / 10_000;

    let before_marketplace_royalty = tok(&env, &t.token).balance(&t.royalty_recipient);
    let before_seller = tok(&env, &t.token).balance(&t.seller);

    t.client.buy(&t.buyer, &id);

    let after_marketplace_royalty = tok(&env, &t.token).balance(&t.royalty_recipient);
    assert_eq!(after_marketplace_royalty, before_marketplace_royalty + marketplace_royalty,
               "marketplace royalty should be used when NFT contract returns no royalty");
    assert_eq!(tok(&env, &t.token).balance(&t.seller),
               before_seller + price - marketplace_royalty,
               "seller should receive price minus marketplace royalty");
}

// ---------------------------------------------------------------------------
// Helpers for multi-token / batch / offer-sweep / ghost-listing tests
// ---------------------------------------------------------------------------

/// Deploy a fresh Stellar asset and mint `amount` to `to`.
fn new_token(env: &Env, admin: &Address, to: &Address, amount: i128) -> Address {
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    StellarAssetClient::new(env, &token).mint(to, &amount);
    token
}

/// Mint `token_id` to the seller on the mock NFT and approve the marketplace.
fn mint_and_approve(t: &TestEnv, token_id: u32) {
    let nft = MockNftClient::new(&t.env, &t.nft);
    nft.init(&t.seller, &token_id);
    nft.approve(
        &t.seller,
        &t.marketplace,
        &token_id,
        &(t.env.ledger().sequence() + 10_000),
    );
}

// ---------------------------------------------------------------------------
// #1096 – multi-token payment support per listing
// ---------------------------------------------------------------------------

#[test]
fn test_listings_in_multiple_payment_tokens() {
    let env = Env::default();
    let t = setup(&env);
    mint_and_approve(&t, 2);

    let usdc = new_token(&env, &t.admin, &t.buyer, 10_000);

    let xlm_id = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128, &t.token);
    let usdc_id = t.client.list(&t.seller, &t.nft, &2u32, &2_000i128, &usdc);
    assert_eq!(t.client.get_listing(&xlm_id).unwrap().payment_token, t.token);
    assert_eq!(t.client.get_listing(&usdc_id).unwrap().payment_token, usdc);

    t.client.buy(&t.buyer, &xlm_id);
    t.client.buy(&t.buyer, &usdc_id);

    // Each sale settles (price and royalty) in its own listing's token.
    assert_eq!(tok(&env, &t.token).balance(&t.seller), 975);
    assert_eq!(tok(&env, &t.token).balance(&t.royalty_recipient), 25);
    assert_eq!(tok(&env, &usdc).balance(&t.seller), 1_950);
    assert_eq!(tok(&env, &usdc).balance(&t.royalty_recipient), 50);
    assert_eq!(tok(&env, &usdc).balance(&t.buyer), 8_000);
}

#[test]
fn test_offer_escrowed_in_listing_token() {
    let env = Env::default();
    let t = setup(&env);
    let usdc = new_token(&env, &t.admin, &t.buyer, 10_000);

    let id = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128, &usdc);
    t.client.make_offer(&t.buyer, &id, &800i128);
    assert_eq!(tok(&env, &usdc).balance(&t.marketplace), 800);
    assert_eq!(tok(&env, &t.token).balance(&t.marketplace), 0);

    t.client.accept_offer(&t.seller, &id, &t.buyer);
    assert_eq!(tok(&env, &usdc).balance(&t.seller), 780);
    assert_eq!(tok(&env, &usdc).balance(&t.royalty_recipient), 20);
}

#[test]
fn test_whitelist_rejects_unlisted_token() {
    let env = Env::default();
    let t = setup(&env);
    let scam = new_token(&env, &t.admin, &t.buyer, 10_000);

    // Default token is whitelisted at init; whitelist is off by default.
    assert!(t.client.is_payment_token_allowed(&t.token));
    assert!(!t.client.is_payment_token_allowed(&scam));
    assert!(!t.client.is_whitelist_enabled());

    t.client.set_whitelist_enabled(&true);
    let res = t.client.try_list(&t.seller, &t.nft, &1u32, &1_000i128, &scam);
    assert_eq!(res, Err(Ok(MarketplaceError::PaymentTokenNotAllowed)));

    t.client.set_payment_token_allowed(&scam, &true);
    assert!(t.client.try_list(&t.seller, &t.nft, &1u32, &1_000i128, &scam).is_ok());

    t.client.set_payment_token_allowed(&scam, &false);
    let res = t.client.try_list(&t.seller, &t.nft, &1u32, &1_000i128, &scam);
    assert_eq!(res, Err(Ok(MarketplaceError::PaymentTokenNotAllowed)));
}

// ---------------------------------------------------------------------------
// #1097 – batch listing, purchase and delisting
// ---------------------------------------------------------------------------

#[test]
fn test_list_batch_and_buy_batch_sweep() {
    let env = Env::default();
    let t = setup(&env);
    mint_and_approve(&t, 2);
    mint_and_approve(&t, 3);
    let usdc = new_token(&env, &t.admin, &t.buyer, 10_000);

    let mut items = Vec::new(&env);
    for (token_id, price, pay) in [(1u32, 100i128, &t.token), (2, 200, &usdc), (3, 300, &t.token)] {
        items.push_back(ListingParams {
            nft_contract: t.nft.clone(),
            token_id,
            price,
            payment_token: pay.clone(),
            expires_at: None,
        });
    }
    let ids = t.client.list_batch(&t.seller, &items);
    assert_eq!(ids.len(), 3);

    t.client.buy_batch(&t.buyer, &ids);

    let nft = MockNftClient::new(&env, &t.nft);
    for token_id in 1u32..=3 {
        assert_eq!(nft.owner_of(&token_id), t.buyer);
    }
    for id in ids.iter() {
        assert!(!t.client.get_listing(&id).unwrap().active);
    }
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), 10_000 - 400);
    assert_eq!(tok(&env, &usdc).balance(&t.buyer), 10_000 - 200);
}

#[test]
fn test_buy_batch_is_all_or_nothing() {
    let env = Env::default();
    let t = setup(&env);
    mint_and_approve(&t, 2);

    let a = t.client.list(&t.seller, &t.nft, &1u32, &100i128, &t.token);
    let b = t.client.list(&t.seller, &t.nft, &2u32, &100i128, &t.token);
    t.client.cancel(&t.seller, &b);

    let res = t.client.try_buy_batch(&t.buyer, &soroban_sdk::vec![&env, a, b]);
    assert_eq!(res, Err(Ok(MarketplaceError::ListingInactive)));

    // The valid listing was not bought and no funds moved.
    assert!(t.client.get_listing(&a).unwrap().active);
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), 10_000);
    assert_eq!(MockNftClient::new(&env, &t.nft).owner_of(&1u32), t.seller);
}

#[test]
fn test_buy_batch_rejects_duplicates_and_bad_sizes() {
    let env = Env::default();
    let t = setup(&env);
    let a = t.client.list(&t.seller, &t.nft, &1u32, &100i128, &t.token);

    let res = t.client.try_buy_batch(&t.buyer, &soroban_sdk::vec![&env, a, a]);
    assert_eq!(res, Err(Ok(MarketplaceError::ListingInactive)));

    let res = t.client.try_buy_batch(&t.buyer, &Vec::new(&env));
    assert_eq!(res, Err(Ok(MarketplaceError::EmptyBatch)));

    let mut too_many = Vec::new(&env);
    for i in 0..=MAX_BATCH_SIZE {
        too_many.push_back(u64::from(i));
    }
    let res = t.client.try_buy_batch(&t.buyer, &too_many);
    assert_eq!(res, Err(Ok(MarketplaceError::BatchTooLarge)));
}

#[test]
fn test_cancel_batch() {
    let env = Env::default();
    let t = setup(&env);
    mint_and_approve(&t, 2);
    let a = t.client.list(&t.seller, &t.nft, &1u32, &100i128, &t.token);
    let b = t.client.list(&t.seller, &t.nft, &2u32, &100i128, &t.token);

    let other = Address::generate(&env);
    let res = t.client.try_cancel_batch(&other, &soroban_sdk::vec![&env, a, b]);
    assert_eq!(res, Err(Ok(MarketplaceError::NotAuthorized)));
    assert!(t.client.get_listing(&a).unwrap().active);

    t.client.cancel_batch(&t.seller, &soroban_sdk::vec![&env, a, b]);
    assert!(!t.client.get_listing(&a).unwrap().active);
    assert!(!t.client.get_listing(&b).unwrap().active);
}

// ---------------------------------------------------------------------------
// #1094 – refund escrowed offers on closed listings
// ---------------------------------------------------------------------------

#[test]
fn test_sweep_offers_refunds_buyers_after_cancel() {
    let env = Env::default();
    let t = setup(&env);
    let buyer2 = Address::generate(&env);
    StellarAssetClient::new(&env, &t.token).mint(&buyer2, &10_000i128);

    let id = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128, &t.token);
    t.client.make_offer(&t.buyer, &id, &600i128);
    t.client.make_offer(&buyer2, &id, &700i128);

    // Cannot sweep while the listing is open.
    let res = t.client.try_sweep_offers(&id, &soroban_sdk::vec![&env, t.buyer.clone()]);
    assert_eq!(res, Err(Ok(MarketplaceError::ListingStillActive)));

    t.client.cancel(&t.seller, &id);

    // Includes a duplicate and a buyer with no offer; neither is double-refunded.
    let stranger = Address::generate(&env);
    let refunded = t.client.sweep_offers(
        &id,
        &soroban_sdk::vec![&env, t.buyer.clone(), buyer2.clone(), t.buyer.clone(), stranger],
    );
    assert_eq!(refunded, 2);
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), 10_000);
    assert_eq!(tok(&env, &t.token).balance(&buyer2), 10_000);
    assert_eq!(tok(&env, &t.token).balance(&t.marketplace), 0);
    assert_eq!(t.client.get_offer(&id, &t.buyer), None);
    assert_eq!(t.client.get_offer(&id, &buyer2), None);
}

#[test]
fn test_sweep_offers_after_expiry_sweep() {
    use soroban_sdk::testutils::Ledger as _;

    let env = Env::default();
    let t = setup(&env);
    let expires_at = env.ledger().sequence() + 10;
    let id = t
        .client
        .list_with_expiry(&t.seller, &t.nft, &1u32, &1_000i128, &t.token, &expires_at);
    t.client.make_offer(&t.buyer, &id, &500i128);

    env.ledger().with_mut(|l| l.sequence_number = expires_at + 1);
    t.client.sweep_expired(&t.seller, &id);

    assert_eq!(t.client.sweep_offers(&id, &soroban_sdk::vec![&env, t.buyer.clone()]), 1);
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), 10_000);
}

// ---------------------------------------------------------------------------
// #1095 – ghost listings
// ---------------------------------------------------------------------------

#[test]
fn test_buy_ghost_listing_fails_without_payment() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128, &t.token);

    // Seller moves the NFT elsewhere after listing.
    let elsewhere = Address::generate(&env);
    MockNftClient::new(&env, &t.nft).set_owner(&1u32, &elsewhere);

    let res = t.client.try_buy(&t.buyer, &id);
    assert_eq!(res, Err(Ok(MarketplaceError::SellerNotOwner)));
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), 10_000);
    assert_eq!(tok(&env, &t.token).balance(&t.seller), 0);
    assert_eq!(MockNftClient::new(&env, &t.nft).owner_of(&1u32), elsewhere);
}

#[test]
fn test_invalidate_ghost_listing_and_filter_queries() {
    let env = Env::default();
    let t = setup(&env);
    mint_and_approve(&t, 2);
    let ghost = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128, &t.token);
    let live = t.client.list(&t.seller, &t.nft, &2u32, &1_000i128, &t.token);

    // Still owned: invalidate is a no-op.
    assert!(!t.client.invalidate_listing(&ghost));

    MockNftClient::new(&env, &t.nft).set_owner(&1u32, &Address::generate(&env));

    // Query filters the ghost even before it is invalidated.
    let page = t.client.get_active_listings(&0u64, &10u32);
    assert_eq!(page.listings.len(), 1);
    assert_eq!(page.listings.get(0).unwrap().id, live);

    assert!(t.client.invalidate_listing(&ghost));
    assert!(!t.client.get_listing(&ghost).unwrap().active);
    assert!(t.client.get_listing(&live).unwrap().active);
}

#[test]
fn test_ghost_listing_offers_can_be_refunded() {
    let env = Env::default();
    let t = setup(&env);
    let id = t.client.list(&t.seller, &t.nft, &1u32, &1_000i128, &t.token);
    t.client.make_offer(&t.buyer, &id, &500i128);

    MockNftClient::new(&env, &t.nft).set_owner(&1u32, &Address::generate(&env));
    let res = t.client.try_accept_offer(&t.seller, &id, &t.buyer);
    assert_eq!(res, Err(Ok(MarketplaceError::SellerNotOwner)));

    t.client.invalidate_listing(&id);
    assert_eq!(t.client.sweep_offers(&id, &soroban_sdk::vec![&env, t.buyer.clone()]), 1);
    assert_eq!(tok(&env, &t.token).balance(&t.buyer), 10_000);
}
