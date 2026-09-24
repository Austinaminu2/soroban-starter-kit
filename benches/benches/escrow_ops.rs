#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
//! Criterion benchmarks for EscrowContract operations.
//!
//! Closes #224 – no benchmark / gas usage tests.
//!
//! Each benchmark measures the CPU instruction count reported by the Soroban
//! test environment, which is a stable proxy for on-chain compute unit (CU)
//! consumption.  The CI threshold check lives in `.github/workflows/bench.yml`.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use soroban_sdk::{
    Address, Env, String,
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
};

use soroban_escrow_template::{EscrowContract, EscrowContractClient};
use soroban_token_template::{TokenContract, TokenContractClient};

fn setup(
    amount: i128,
) -> (
    Env,
    EscrowContractClient<'static>,
    Address,
    Address,
    Address,
    Address,
    u32,
) {
    // SAFETY: we leak the Env so that the 'static lifetime is satisfied for
    // the client references returned from this helper.  This is acceptable in
    // benchmark code where the process exits after each measurement.
    let env: &'static Env = Box::leak(Box::new(Env::default()));
    env.mock_all_auths();

    let token_admin = Address::generate(env);
    let buyer = Address::generate(env);
    let seller = Address::generate(env);
    let arbiter = Address::generate(env);
    let deadline = env.ledger().sequence() + 500;

    // Deploy token and mint to buyer
    let token_addr = env.register_contract(None, TokenContract);
    let token = TokenContractClient::new(env, &token_addr);
    token.initialize(
        &token_admin,
        &String::from_str(env, "Bench Token"),
        &String::from_str(env, "BT"),
        &18u32,
        &None,
    );
    token.mint(&buyer, &amount);

    // Deploy escrow
    let escrow_addr = env.register_contract(None, EscrowContract);
    let escrow = EscrowContractClient::new(env, &escrow_addr);

    (
        env.clone(),
        escrow,
        buyer,
        seller,
        arbiter,
        token_addr,
        deadline,
    )
}

fn bench_initialize(c: &mut Criterion) {
    c.bench_function("escrow::initialize", |b| {
        b.iter(|| {
            let env = Env::default();
            env.mock_all_auths();
            let admin = Address::generate(&env);
            let buyer = Address::generate(&env);
            let seller = Address::generate(&env);
            let arbiter = Address::generate(&env);
            let token_addr = Address::generate(&env);
            let deadline = env.ledger().sequence() + 500;

            let escrow_addr = env.register_contract(None, EscrowContract);
            let escrow = EscrowContractClient::new(&env, &escrow_addr);
            escrow.initialize(
                black_box(&admin),
                black_box(&buyer),
                black_box(&seller),
                black_box(&arbiter),
                black_box(&token_addr),
                black_box(&1_000i128),
                black_box(&deadline),
                black_box(&0u32),
                black_box(&None),
            );
        });
    });
}

fn bench_fund(c: &mut Criterion) {
    c.bench_function("escrow::fund", |b| {
        b.iter(|| {
            let env = Env::default();
            env.mock_all_auths();

            let token_admin = Address::generate(&env);
            let admin = Address::generate(&env);
            let buyer = Address::generate(&env);
            let seller = Address::generate(&env);
            let arbiter = Address::generate(&env);
            let deadline = env.ledger().sequence() + 500;

            let sac = env.register_stellar_asset_contract_v2(token_admin);
            let token_addr = sac.address();
            StellarAssetClient::new(&env, &token_addr).mint(&buyer, &1_000i128);

            let escrow_addr = env.register_contract(None, EscrowContract);
            let escrow = EscrowContractClient::new(&env, &escrow_addr);
            escrow.initialize(
                &admin,
                &buyer,
                &seller,
                &arbiter,
                &token_addr,
                &1_000i128,
                &deadline,
                &0u32,
                &None,
            );

            escrow.fund();
        });
    });
}

fn bench_approve_delivery(c: &mut Criterion) {
    c.bench_function("escrow::approve_delivery", |b| {
        b.iter(|| {
            let env = Env::default();
            env.mock_all_auths();

            let token_admin = Address::generate(&env);
            let admin = Address::generate(&env);
            let buyer = Address::generate(&env);
            let seller = Address::generate(&env);
            let arbiter = Address::generate(&env);
            let deadline = env.ledger().sequence() + 500;

            let sac = env.register_stellar_asset_contract_v2(token_admin);
            let token_addr = sac.address();
            StellarAssetClient::new(&env, &token_addr).mint(&buyer, &1_000i128);

            let escrow_addr = env.register_contract(None, EscrowContract);
            let escrow = EscrowContractClient::new(&env, &escrow_addr);
            escrow.initialize(
                &admin,
                &buyer,
                &seller,
                &arbiter,
                &token_addr,
                &1_000i128,
                &deadline,
                &0u32,
                &None,
            );
            escrow.fund();
            escrow.mark_delivered();

            escrow.approve_delivery();
        });
    });
}

fn bench_resolve_dispute(c: &mut Criterion) {
    c.bench_function("escrow::resolve_dispute", |b| {
        b.iter(|| {
            let env = Env::default();
            env.mock_all_auths();

            let token_admin = Address::generate(&env);
            let admin = Address::generate(&env);
            let buyer = Address::generate(&env);
            let seller = Address::generate(&env);
            let arbiter = Address::generate(&env);
            let deadline = env.ledger().sequence() + 500;

            let sac = env.register_stellar_asset_contract_v2(token_admin);
            let token_addr = sac.address();
            StellarAssetClient::new(&env, &token_addr).mint(&buyer, &1_000i128);

            let escrow_addr = env.register_contract(None, EscrowContract);
            let escrow = EscrowContractClient::new(&env, &escrow_addr);
            escrow.initialize(
                &admin,
                &buyer,
                &seller,
                &arbiter,
                &token_addr,
                &1_000i128,
                &deadline,
                &0u32,
                &None,
            );
            escrow.fund();

            escrow.resolve_dispute(black_box(&arbiter), black_box(&true));
        });
    });
}

fn bench_full_lifecycle(c: &mut Criterion) {
    c.bench_function("escrow::full_lifecycle", |b| {
        b.iter(|| {
            let env = Env::default();
            env.mock_all_auths();

            let token_admin = Address::generate(&env);
            let admin = Address::generate(&env);
            let buyer = Address::generate(&env);
            let seller = Address::generate(&env);
            let arbiter = Address::generate(&env);
            let deadline = env.ledger().sequence() + 500;

            let sac = env.register_stellar_asset_contract_v2(token_admin);
            let token_addr = sac.address();
            StellarAssetClient::new(&env, &token_addr).mint(&buyer, &1_000i128);

            let escrow_addr = env.register_contract(None, EscrowContract);
            let escrow = EscrowContractClient::new(&env, &escrow_addr);
            escrow.initialize(
                &admin,
                &buyer,
                &seller,
                &arbiter,
                &token_addr,
                &1_000i128,
                &deadline,
                &0u32,
                &None,
            );

            // Full lifecycle: fund → mark_delivered → approve_delivery
            escrow.fund();
            escrow.mark_delivered();
            escrow.approve_delivery();
        });
    });
}

criterion_group!(
    benches,
    bench_initialize,
    bench_fund,
    bench_approve_delivery,
    bench_resolve_dispute,
    bench_full_lifecycle,
);
criterion_main!(benches);
