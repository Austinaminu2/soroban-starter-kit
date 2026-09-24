// `#[contracttype]` generates undocumented public associated items.
#![allow(missing_docs)]

use soroban_sdk::{Address, contracttype};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin address (instance).
    Admin,
    /// Default payment token address, set at initialization (instance).
    PaymentToken,
    /// Royalty in basis points, e.g. 250 = 2.5 % (instance).
    RoyaltyBps,
    /// Royalty recipient address (instance).
    RoyaltyRecipient,
    /// Next listing ID counter (instance).
    NextListingId,
    /// Per-listing details (persistent).
    Listing(u64),
    /// Escrowed offer amount for (listing_id, buyer) (persistent).
    Offer(u64, Address),
    /// Whether the payment-token whitelist is enforced on new listings (instance).
    WhitelistEnabled,
    /// Whether a payment token is whitelisted by the admin (persistent).
    AllowedToken(Address),
    /// Reentrancy guard held while a sale or batch operation is in progress (instance).
    Locked,
}

/// State of a single NFT listing.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    /// The NFT contract address.
    pub nft_contract: Address,
    /// The token ID being sold.
    pub token_id: u32,
    /// The seller.
    pub seller: Address,
    /// Asking price in `payment_token` units.
    pub price: i128,
    /// Token the seller accepts as payment for this listing. Purchases, offers
    /// and royalties on this listing are all settled in this token.
    pub payment_token: Address,
    /// Whether the listing is still open.
    pub active: bool,
    /// Optional ledger sequence after which the listing can no longer be bought.
    pub expires_at: Option<u32>,
}

/// A listing paired with its ID, as returned by [`super::MarketplaceContract::get_active_listings`].
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ListingEntry {
    /// The listing ID.
    pub id: u64,
    /// The listing itself.
    pub listing: Listing,
}

/// One page of results from [`super::MarketplaceContract::get_active_listings`].
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ListingPage {
    /// Active listings found in this page, in ascending ID order.
    pub listings: soroban_sdk::Vec<ListingEntry>,
    /// The cursor to pass to the next call to continue scanning, or `None`
    /// if the end of the listing range has been reached.
    pub next_cursor: Option<u64>,
}

/// Parameters for a single listing created through
/// [`super::MarketplaceContract::list_batch`].
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ListingParams {
    /// The NFT contract address.
    pub nft_contract: Address,
    /// The token ID being sold.
    pub token_id: u32,
    /// Asking price in `payment_token` units.
    pub price: i128,
    /// Token the seller accepts as payment.
    pub payment_token: Address,
    /// Optional ledger sequence after which the listing can no longer be bought.
    pub expires_at: Option<u32>,
}
