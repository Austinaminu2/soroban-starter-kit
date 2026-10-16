use soroban_sdk::{Address, Env, Symbol};

pub fn initialized(env: &Env, admin: &Address, staleness_threshold: u32) {
    env.events().publish(
        (Symbol::new(env, "initialized"), admin.clone()),
        staleness_threshold,
    );
}

pub fn price_updated(env: &Env, admin: &Address, price: i128, ledger: u32) {
    env.events().publish(
        (Symbol::new(env, "price_updated"), admin.clone()),
        (price, ledger),
    );
}

pub fn price_submitted(env: &Env, publisher: &Address, price: i128, timestamp: u64) {
    env.events().publish(
        (Symbol::new(env, "price_submitted"), publisher.clone()),
        (price, timestamp),
    );
}

pub fn asset_price_updated(
    env: &Env,
    admin: &Address,
    asset: &Symbol,
    price: i128,
    decimals: u32,
    ledger: u32,
) {
    env.events().publish(
        (Symbol::new(env, "asset_price_updated"), admin.clone()),
        (asset.clone(), price, decimals, ledger),
    );
}
