use soroban_sdk::{Address, Env, Symbol};

pub fn started(env: &Env, seller: &Address, start_price: i128, deadline: u32) {
    env.events().publish(
        (Symbol::new(env, "started"), seller.clone()),
        (start_price, deadline),
    );
}

pub fn bid_placed(env: &Env, bidder: &Address, amount: i128) {
    env.events()
        .publish((Symbol::new(env, "bid_placed"), bidder.clone()), amount);
}

pub fn outbid(
    env: &Env,
    outbid_bidder: &Address,
    outbid_amount: i128,
    new_highest_bid: i128,
) {
    env.events().publish(
        (Symbol::new(env, "outbid"), outbid_bidder.clone()),
        (outbid_amount, new_highest_bid),
    );
}

pub fn refund_queued(env: &Env, bidder: &Address, amount: i128) {
    env.events()
        .publish((Symbol::new(env, "refund_queued"), bidder.clone()), amount);
}

pub fn ended(env: &Env, winner: &Address, amount: i128) {
    env.events()
        .publish((Symbol::new(env, "ended"), winner.clone()), amount);
}

pub fn ended_no_bids(env: &Env) {
    env.events()
        .publish((Symbol::new(env, "ended_no_bids"),), ());
}

pub fn ended_reserve_not_met(
    env: &Env,
    highest_bidder: &Address,
    highest_bid: i128,
    reserve_price: i128,
) {
    env.events().publish(
        (
            Symbol::new(env, "ended_reserve_not_met"),
            highest_bidder.clone(),
        ),
        (highest_bid, reserve_price),
    );
}

pub fn withdrawn(env: &Env, bidder: &Address, amount: i128) {
    env.events()
        .publish((Symbol::new(env, "withdrawn"), bidder.clone()), amount);
}

pub fn deadline_extended(env: &Env, new_deadline: u32) {
    env.events()
        .publish((Symbol::new(env, "deadline_extended"),), new_deadline);
}

pub fn cancelled(env: &Env, seller: &Address) {
    env.events()
        .publish((Symbol::new(env, "cancelled"), seller.clone()), ());
}
