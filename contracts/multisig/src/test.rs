#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![cfg(test)]

use super::*;
use soroban_sdk::{
    Address, Env, FromVal, IntoVal, Symbol, contract, contractimpl,
    testutils::{Address as _, Events as _, Ledger as _},
    vec,
};

#[contract]
pub struct CounterContract;

#[contractimpl]
impl CounterContract {
    pub fn increment(env: Env, amount: u32) -> u32 {
        let current = Self::get(env.clone());
        let next = current + amount;
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "count"), &next);
        next
    }

    pub fn get(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "count"))
            .unwrap_or(0)
    }
}

/// Helper: create a 2-of-3 multisig with uniform weights (no weights param).
fn create_multisig<'a>(
    env: &'a Env,
) -> (
    MultisigContractClient<'a>,
    Address,
    Address,
    Address,
    Address,
) {
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let carol = Address::generate(env);
    let contract_address = env.register_contract(None, MultisigContract);
    let client = MultisigContractClient::new(env, &contract_address);

    client.initialize(
        &vec![env, alice.clone(), bob.clone(), carol.clone()],
        &2,
        &None,
    );

    (client, alice, bob, carol, contract_address)
}

#[test]
fn initialize_stores_signers_and_threshold() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, alice, bob, carol, contract_address) = create_multisig(&env);

    assert_eq!(client.get_threshold(), Some(2));
    assert_eq!(client.get_signers(), vec![&env, alice, bob, carol]);
    assert_eq!(
        env.events().all(),
        vec![
            &env,
            (
                contract_address,
                (Symbol::new(&env, "initialized"), 2u32).into_val(&env),
                3u32.into_val(&env),
            )
        ]
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn initialize_rejects_zero_threshold() {
    let env = Env::default();
    env.mock_all_auths();

    let alice = Address::generate(&env);
    let contract_address = env.register_contract(None, MultisigContract);
    let client = MultisigContractClient::new(&env, &contract_address);

    client.initialize(&vec![&env, alice], &0, &None);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn initialize_rejects_duplicate_signers() {
    let env = Env::default();
    env.mock_all_auths();

    let alice = Address::generate(&env);
    let contract_address = env.register_contract(None, MultisigContract);
    let client = MultisigContractClient::new(&env, &contract_address);

    client.initialize(&vec![&env, alice.clone(), alice], &1, &None);
}

#[test]
fn add_signer_with_threshold_approvals_updates_signer_set() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, carol, _) = create_multisig(&env);
    let dave = Address::generate(&env);

    client.add_signer(&vec![&env, alice.clone(), bob.clone()], &dave, &1, &3);

    assert_eq!(client.get_threshold(), Some(3));
    assert_eq!(client.get_signers(), vec![&env, alice, bob, carol, dave]);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn add_signer_rejects_insufficient_approvals() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, _, _, _) = create_multisig(&env);
    let dave = Address::generate(&env);

    client.add_signer(&vec![&env, alice], &dave, &1, &2);
}

#[test]
fn remove_signer_with_threshold_approvals_updates_signer_set() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, carol, _) = create_multisig(&env);

    client.remove_signer(&vec![&env, alice.clone(), bob.clone()], &carol, &2);

    assert_eq!(client.get_threshold(), Some(2));
    assert_eq!(client.get_signers(), vec![&env, alice, bob]);
    assert!(!client.is_signer(&carol));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn remove_signer_rejects_threshold_above_remaining_signers() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, carol, _) = create_multisig(&env);

    client.remove_signer(&vec![&env, alice, bob], &carol, &3);
}

#[test]
fn propose_transaction_stores_transaction_and_auto_signature() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, _, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 7u32.into_val(&env)],
        &100u32,
    );

    let transaction = client.get_transaction(&tx_id).expect("transaction exists");
    assert_eq!(tx_id, 0);
    assert_eq!(transaction.proposer, alice.clone());
    assert_eq!(transaction.target, target);
    assert_eq!(transaction.signatures, vec![&env, alice]);
    assert!(!transaction.executed);
    assert_eq!(client.signature_count(&tx_id), Some(1));
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn non_signer_cannot_propose_transaction() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _, _, _, _) = create_multisig(&env);
    let outsider = Address::generate(&env);
    let target = env.register_contract(None, CounterContract);

    client.propose_transaction(
        &outsider,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &100u32,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn signer_cannot_sign_same_transaction_twice() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, _, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);
    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &100u32,
    );

    client.sign_transaction(&alice, &tx_id);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn execute_rejects_when_threshold_not_met() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, _, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);
    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &100u32,
    );

    client.execute_transaction(&tx_id);
}

#[test]
fn execute_runs_target_call_once_when_threshold_met() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);
    let counter = CounterContractClient::new(&env, &target);
    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 5u32.into_val(&env)],
        &100u32,
    );

    client.sign_transaction(&bob, &tx_id);
    let result = client.execute_transaction(&tx_id);
    let value = u32::from_val(&env, &result);

    assert_eq!(value, 5);
    assert_eq!(counter.get(), 5);
    assert!(
        client
            .get_transaction(&tx_id)
            .expect("transaction exists")
            .executed
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn execute_rejects_second_execution() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);
    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 5u32.into_val(&env)],
        &100u32,
    );

    client.sign_transaction(&bob, &tx_id);
    client.execute_transaction(&tx_id);
    client.execute_transaction(&tx_id);
}

// ── #715 proposal expiry tests ───────────────────────────────────────────────

#[test]
fn proposal_stores_expiry_ledger() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, _, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &50u32,
    );

    let tx = client.get_transaction(&tx_id).expect("transaction exists");
    assert_eq!(tx.expiry_ledger, env.ledger().sequence() + 50);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn sign_after_expiry_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &10u32,
    );

    // Advance ledger past expiry
    env.ledger().with_mut(|l| l.sequence_number += 11);
    client.sign_transaction(&bob, &tx_id);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn execute_after_expiry_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &10u32,
    );
    client.sign_transaction(&bob, &tx_id);

    // Advance ledger past expiry
    env.ledger().with_mut(|l| l.sequence_number += 11);
    client.execute_transaction(&tx_id);
}

#[test]
fn cleanup_expired_removes_proposal_and_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, _, _, contract_address) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &10u32,
    );

    // Advance past expiry
    env.ledger().with_mut(|l| l.sequence_number += 11);
    client.cleanup_expired(&tx_id);

    // Proposal should be gone
    assert_eq!(client.get_transaction(&tx_id), None);

    // expired event emitted
    let found = env.events().all().iter().any(|(_, topics, _)| {
        topics == (Symbol::new(&env, "expired"), tx_id).into_val(&env)
    });
    assert!(found, "expired event not emitted");
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn cleanup_not_yet_expired_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, _, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &100u32,
    );

    // Not yet expired — should fail
    client.cleanup_expired(&tx_id);
}

// ── #825 weighted voting tests ───────────────────────────────────────────────

/// Helper: weighted multisig where alice=3, bob=2, carol=1; threshold=4.
fn create_weighted_multisig<'a>(
    env: &'a Env,
) -> (
    MultisigContractClient<'a>,
    Address,
    Address,
    Address,
) {
    let alice = Address::generate(env);
    let bob = Address::generate(env);
    let carol = Address::generate(env);
    let contract_address = env.register_contract(None, MultisigContract);
    let client = MultisigContractClient::new(env, &contract_address);

    let weights = vec![
        env,
        SignerWeight { signer: alice.clone(), weight: 3 },
        SignerWeight { signer: bob.clone(),   weight: 2 },
        SignerWeight { signer: carol.clone(), weight: 1 },
    ];
    // threshold = 4; alice alone (3) is not enough; alice+bob (5) is enough.
    client.initialize(
        &vec![env, alice.clone(), bob.clone(), carol.clone()],
        &4,
        &Some(weights),
    );

    (client, alice, bob, carol)
}

#[test]
fn weighted_initialize_stores_weights() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, carol) = create_weighted_multisig(&env);

    assert_eq!(client.get_signer_weight(&alice), 3);
    assert_eq!(client.get_signer_weight(&bob),   2);
    assert_eq!(client.get_signer_weight(&carol),  1);
    assert_eq!(client.get_threshold(), Some(4));
}

#[test]
fn weighted_threshold_met_with_two_heavy_signers() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, _carol) = create_weighted_multisig(&env);
    let target = env.register_contract(None, CounterContract);
    let counter = CounterContractClient::new(&env, &target);

    // alice proposes (accumulated = 3), bob signs (accumulated = 5 >= 4)
    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 10u32.into_val(&env)],
        &100u32,
    );
    client.sign_transaction(&bob, &tx_id);
    client.execute_transaction(&tx_id);

    assert_eq!(counter.get(), 10);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn weighted_threshold_not_met_with_single_heavy_signer() {
    let env = Env::default();
    env.mock_all_auths();
    // alice (weight 3) alone does not meet threshold 4
    let (client, alice, _, _) = create_weighted_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &100u32,
    );
    // should panic: weight 3 < threshold 4
    client.execute_transaction(&tx_id);
}

#[test]
fn weighted_accumulated_weight_stored_on_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, _) = create_weighted_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    let tx_id = client.propose_transaction(
        &alice,
        &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &100u32,
    );
    let tx_after_propose = client.get_transaction(&tx_id).unwrap();
    assert_eq!(tx_after_propose.accumulated_weight, 3); // alice weight

    client.sign_transaction(&bob, &tx_id);
    let tx_after_bob = client.get_transaction(&tx_id).unwrap();
    assert_eq!(tx_after_bob.accumulated_weight, 5); // alice(3) + bob(2)
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn initialize_rejects_zero_weight() {
    let env = Env::default();
    env.mock_all_auths();
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let contract_address = env.register_contract(None, MultisigContract);
    let client = MultisigContractClient::new(&env, &contract_address);

    let weights = vec![
        &env,
        SignerWeight { signer: alice.clone(), weight: 0 }, // zero weight → error
        SignerWeight { signer: bob.clone(),   weight: 1 },
    ];
    client.initialize(&vec![&env, alice, bob], &1, &Some(weights));
}

// ── #826 batch execution tests ───────────────────────────────────────────────

#[test]
fn execute_batch_executes_all_ready_proposals() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);
    let counter = CounterContractClient::new(&env, &target);

    // Propose two transactions, both signed by alice+bob (threshold=2).
    let tx0 = client.propose_transaction(
        &alice, &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &100u32,
    );
    client.sign_transaction(&bob, &tx0);

    let tx1 = client.propose_transaction(
        &alice, &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 2u32.into_val(&env)],
        &100u32,
    );
    client.sign_transaction(&bob, &tx1);

    let executed = client.execute_batch(&vec![&env, tx0, tx1]);

    assert_eq!(executed.len(), 2);
    assert_eq!(counter.get(), 3); // 1 + 2
}

#[test]
fn execute_batch_skips_already_executed_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    let tx_id = client.propose_transaction(
        &alice, &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 5u32.into_val(&env)],
        &100u32,
    );
    client.sign_transaction(&bob, &tx_id);
    // Execute individually first.
    client.execute_transaction(&tx_id);

    // A second proposal that can still execute.
    let tx1 = client.propose_transaction(
        &alice, &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &100u32,
    );
    client.sign_transaction(&bob, &tx1);

    // Batch: already-executed tx_id is skipped, tx1 executes.
    let executed = client.execute_batch(&vec![&env, tx_id, tx1]);
    assert_eq!(executed.len(), 1);
    assert_eq!(executed.get(0).unwrap(), tx1);
}

#[test]
fn execute_batch_skips_nonexistent_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    let tx_id = client.propose_transaction(
        &alice, &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 7u32.into_val(&env)],
        &100u32,
    );
    client.sign_transaction(&bob, &tx_id);

    // Include a nonexistent ID (999) alongside valid tx_id.
    let executed = client.execute_batch(&vec![&env, 999u64, tx_id]);
    // 999 skipped, tx_id executed.
    assert_eq!(executed.len(), 1);
    assert_eq!(executed.get(0).unwrap(), tx_id);
}

#[test]
fn execute_batch_skips_threshold_not_met_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    // tx0: only alice signed (weight 1, threshold 2) — not ready.
    let tx0 = client.propose_transaction(
        &alice, &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &100u32,
    );

    // tx1: alice+bob signed — ready.
    let tx1 = client.propose_transaction(
        &alice, &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 3u32.into_val(&env)],
        &100u32,
    );
    client.sign_transaction(&bob, &tx1);

    let executed = client.execute_batch(&vec![&env, tx0, tx1]);
    assert_eq!(executed.len(), 1);
    assert_eq!(executed.get(0).unwrap(), tx1);
}

#[test]
fn execute_batch_emits_batch_executed_event() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, alice, bob, _, _) = create_multisig(&env);
    let target = env.register_contract(None, CounterContract);

    let tx_id = client.propose_transaction(
        &alice, &target,
        &Symbol::new(&env, "increment"),
        &vec![&env, 1u32.into_val(&env)],
        &100u32,
    );
    client.sign_transaction(&bob, &tx_id);

    client.execute_batch(&vec![&env, tx_id]);

    let found = env
        .events()
        .all()
        .iter()
        .any(|(_, topics, _)| topics == (Symbol::new(&env, "batch_executed"),).into_val(&env));
    assert!(found, "batch_executed event not emitted");
}
