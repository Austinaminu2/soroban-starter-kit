#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
//! Criterion benchmarks for TokenContract operations.
//!
//! Closes #224 – no benchmark / gas usage tests.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use soroban_sdk::{Address, Env, String, testutils::Address as _};

use soroban_token_template::{TokenContract, TokenContractClient};

fn deploy_token<'a>(env: &'a Env, admin: &Address) -> TokenContractClient<'a> {
    let addr = env.register_contract(None, TokenContract);
    let client = TokenContractClient::new(env, &addr);
    client.initialize(
        admin,
        &String::from_str(env, "Bench Token"),
        &String::from_str(env, "BT"),
        &18u32,
        &None,
    );
    client
}

fn bench_mint(c: &mut Criterion) {
    c.bench_function("token::mint", |b| {
        b.iter(|| {
            let env = Env::default();
            env.mock_all_auths();
            let admin = Address::generate(&env);
            let user = Address::generate(&env);
            let token = deploy_token(&env, &admin);
            token.mint(black_box(&user), black_box(&1_000i128));
        });
    });
}

fn bench_transfer(c: &mut Criterion) {
    c.bench_function("token::transfer", |b| {
        b.iter(|| {
            let env = Env::default();
            env.mock_all_auths();
            let admin = Address::generate(&env);
            let from = Address::generate(&env);
            let to = Address::generate(&env);
            let token = deploy_token(&env, &admin);
            token.mint(&from, &1_000i128);
            token.transfer(black_box(&from), black_box(&to), black_box(&500i128));
        });
    });
}

fn bench_approve_and_transfer_from(c: &mut Criterion) {
    c.bench_function("token::approve_and_transfer_from", |b| {
        b.iter(|| {
            let env = Env::default();
            env.mock_all_auths();
            let admin = Address::generate(&env);
            let owner = Address::generate(&env);
            let spender = Address::generate(&env);
            let recipient = Address::generate(&env);
            let expiration = env.ledger().sequence() + 100;
            let token = deploy_token(&env, &admin);
            token.mint(&owner, &1_000i128);
            token.approve(
                black_box(&owner),
                black_box(&spender),
                black_box(&500i128),
                black_box(&expiration),
            );
            token.transfer_from(
                black_box(&spender),
                black_box(&owner),
                black_box(&recipient),
                black_box(&200i128),
            );
        });
    });
}

fn bench_burn(c: &mut Criterion) {
    c.bench_function("token::burn", |b| {
        b.iter(|| {
            let env = Env::default();
            env.mock_all_auths();
            let admin = Address::generate(&env);
            let user = Address::generate(&env);
            let token = deploy_token(&env, &admin);
            token.mint(&user, &1_000i128);
            token.burn(black_box(&user), black_box(&300i128));
        });
    });
}

criterion_group!(
    benches,
    bench_mint,
    bench_transfer,
    bench_approve_and_transfer_from,
    bench_burn
);
criterion_main!(benches);
