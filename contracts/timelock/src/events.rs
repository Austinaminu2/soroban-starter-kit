use soroban_sdk::{Address, Env, Symbol};

pub fn initialized(
    env: &Env,
    admin: &Address,
    beneficiary: &Address,
    release_ledger: u32,
    amount: i128,
) {
    env.events().publish(
        (
            Symbol::new(env, "initialized"),
            admin.clone(),
            beneficiary.clone(),
        ),
        (release_ledger, amount),
    );
}

pub fn released(env: &Env, beneficiary: &Address, amount: i128) {
    env.events()
        .publish((Symbol::new(env, "released"), beneficiary.clone()), amount);
}

pub fn cancelled(env: &Env, admin: &Address, amount: i128) {
    env.events()
        .publish((Symbol::new(env, "cancelled"), admin.clone()), amount);
}

pub fn beneficiary_reassigned(env: &Env, admin: &Address, old_beneficiary: &Address, new_beneficiary: &Address) {
    env.events().publish(
        (
            Symbol::new(env, "beneficiary_reassigned"),
            admin.clone(),
            old_beneficiary.clone(),
            new_beneficiary.clone(),
        ),
        (),
    );
}

pub fn deposited(env: &Env, staking_contract: &Address, amount: i128) {
    env.events().publish(
        (Symbol::new(env, "deposited"), staking_contract.clone()),
        amount,
    );
}

pub fn withdrawn(env: &Env, staking_contract: &Address, amount: i128) {
    env.events().publish(
        (Symbol::new(env, "withdrawn"), staking_contract.clone()),
        amount,
    );
}

pub fn yield_distributed(
    env: &Env,
    beneficiary: &Address,
    principal: i128,
    yield_recipient: &Address,
    yield_amount: i128,
) {
    env.events().publish(
        (
            Symbol::new(env, "yield_distributed"),
            beneficiary.clone(),
            yield_recipient.clone(),
        ),
        (principal, yield_amount),
    );
}