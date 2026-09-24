use soroban_sdk::{Address, Bytes, Env, Symbol};

pub fn root_set(env: &Env, root: &Bytes) {
    env.events()
        .publish((Symbol::new(env, "root_set"),), root.clone());
}

pub fn claimed(env: &Env, recipient: &Address, amount: i128) {
    env.events()
        .publish((Symbol::new(env, "claimed"),), (recipient.clone(), amount));
}
