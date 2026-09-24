# Front-Running Risk Assessment

This document assesses front-running/MEV risks specific to the lottery, auction, and marketplace contracts, and documents mitigations applied or recommended.

## Lottery Contract

### Risk: Reveal-Timing Bias (#776)

**Description:** After the admin commits to a hashed secret, they have an advantage in knowing the reveal timing. If the reveal deadline is too short, participants may not have time to verify the randomness.

**Mitigations Applied:**
- Commit-reveal scheme ensures the admin cannot change the secret after commitment
- Reveal deadline is set at commit time and cannot be shortened
- Participants can claim refunds if the deadline passes without a draw

**Recommendations:**
- Set reveal deadlines sufficiently far in the future (e.g., 100+ ledgers)
- Consider adding a minimum reveal deadline enforced by the contract

### Risk: Winner Selection Manipulation

**Description:** The admin could potentially manipulate the winner selection by choosing a specific secret/salt combination.

**Mitigations Applied:**
- The commit hash is stored on-chain before the reveal
- The reveal must match the committed hash exactly
- Winner selection uses SHA-256 entropy derived from the secret, salt, and ledger sequence

**Recommendations:**
- Consider using a verifiable random function (VRF) for higher assurance
- Document the entropy derivation process for transparency

---

## Auction Contract

### Risk: Bid Sniping

**Description:** Bidders can front-run other bids by observing pending transactions and placing higher bids first.

**Mitigations Applied:**
- Anti-sniping extension window extends the deadline when bids arrive near the end
- Minimum increment ensures bids must be meaningfully higher
- Highest bidder is tracked and previous bidders receive refunds

**Recommendations:**
- Set an appropriate extension window (e.g., 10 ledgers)
- Consider adding a minimum bid increment that scales with the current price

### Risk: Reserve Price Front-Running

**Description:** If the reserve price is public, bidders could coordinate to bid just above it.

**Mitigations Applied:**
- Reserve price is optional and can be kept private until auction ends
- Settled flag prevents multiple settlements

**Recommendations:**
- Consider allowing sealed-bid auctions for high-value items
- Document the trade-offs of public vs. private reserve prices

---

## Marketplace Contract

### Risk: Offer Front-Running

**Description:** A buyer could observe a pending offer acceptance and front-run with a higher offer.

**Mitigations Applied:**
- Only the seller can accept offers
- Offers are escrowed and can be cancelled by the buyer
- Listing state is updated before external calls (checks-effects-interactions)

**Recommendations:**
- Consider adding a minimum offer increase requirement
- Document the offer replacement behavior clearly

### Risk: Price Change Before Purchase (#1101)

**Description:** A seller could cancel and re-list at a higher price (or otherwise change the effective price) between the time a buyer signs a `buy` transaction and the time it executes.

**Mitigations Applied:**
- `buy(buyer, listing_id, max_price)` requires the buyer to state the maximum price they will pay
- The purchase is rejected with `PriceExceedsMax` if `listing.price > max_price`; no funds or NFT move

### Risk: Listing Expiry Manipulation

**Description:** A buyer could observe a pending listing creation and front-run with a purchase before the seller can set an expiry.

**Mitigations Applied:**
- Listings are active immediately upon creation
- Expiry is set at listing time and cannot be shortened
- sweep_expired allows sellers to reclaim expired listings

**Recommendations:**
- Consider allowing sellers to set an expiry after listing creation
- Document the expiry behavior clearly

---

## General Recommendations

1. **Transaction Ordering:** Consider using commit-reveal schemes for sensitive operations
2. **MEV Protection:** Document known MEV vectors and mitigations
3. **Monitoring:** Add event logging for suspicious activity patterns
4. **Documentation:** Clearly document front-running risks in contract READMEs

---

See also: [docs/security.md](security.md) for general security considerations.
