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
