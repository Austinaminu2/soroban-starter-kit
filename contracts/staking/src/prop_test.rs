#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#![cfg(test)]

use proptest::prelude::*;
use soroban_sdk::{
    Address, Env,
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
};

use crate::{REWARD_SCALE, StakingContract, StakingContractClient};

fn setup_staking<'a>(env: &'a Env) -> (StakingContractClient<'a>, Address, Address, Address) {
    let admin = Address::generate(env);
    let sac_admin = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(sac_admin.clone());
    let stake_token_addr = sac.address();

    let sac2 = env.register_stellar_asset_contract_v2(sac_admin);
    let reward_token_addr = sac2.address();

    let staking_addr = env.register_contract(None, StakingContract);
    let client = StakingContractClient::new(env, &staking_addr);

    let slash_destination = Address::generate(env);
    client.initialize(
        &admin,
        &stake_token_addr,
        &reward_token_addr,
        &0u32,
        &slash_destination,
    );

    (client, stake_token_addr, reward_token_addr, admin)
}

proptest! {
    /// Property: Total rewards distributed can never exceed total rewards added
    /// Closes #961 – staking reward-per-token accrual invariant
    #[test]
    fn prop_rewards_out_never_exceed_rewards_in(
        stakes in proptest::collection::vec(100i128..=10_000i128, 1..=5),
        rewards_added in 1_000i128..=100_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let (client, stake_token, reward_token, admin) = setup_staking(&env);

        // Add rewards
        StellarAssetClient::new(&env, &reward_token).mint(&admin, &rewards_added);
        let _ = client.try_add_rewards(&rewards_added);

        let mut stakers = vec![];
        let mut total_claimed = 0i128;

        // Multiple stakers stake tokens
        for stake_amount in stakes {
            let staker = Address::generate(&env);
            StellarAssetClient::new(&env, &stake_token).mint(&staker, &stake_amount);
            if client.try_stake(&staker, &stake_amount).is_ok() {
                stakers.push(staker);
            }
        }

        // Claim rewards
        for staker in &stakers {
            let claimed_result = client.try_claim_rewards(staker);
            if let Ok(amount) = claimed_result {
                total_claimed += amount;
            }
        }

        // Invariant: total claimed <= total added
        prop_assert!(total_claimed <= rewards_added,
            "Rewards claimed exceed rewards added: added={}, claimed={}",
            rewards_added, total_claimed);
    }

    /// Property: Stake balance never goes negative
    #[test]
    fn prop_stake_never_negative(
        stake_amount in 100i128..=10_000i128,
        unstake_amount in 1i128..=20_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let (client, stake_token, _, _) = setup_staking(&env);
        let staker = Address::generate(&env);

        StellarAssetClient::new(&env, &stake_token).mint(&staker, &stake_amount);
        let _ = client.try_stake(&staker, &stake_amount);

        let _ = client.try_unstake(&staker, &unstake_amount);

        let final_stake = client.get_stake(staker);
        prop_assert!(final_stake >= 0, "Stake went negative: {}", final_stake);
    }

    /// Property: Reward calculation is deterministic
    #[test]
    fn prop_reward_calculation_deterministic(
        stake in 1i128..=1_000_000i128,
        rpt in 0i128..=1_000_000i128,
        paid in 0i128..=1_000_000i128,
        accrued in 0i128..=1_000_000i128,
    ) {
        use crate::calculate_earned;

        let result1 = calculate_earned(stake, rpt, paid, accrued);
        let result2 = calculate_earned(stake, rpt, paid, accrued);

        prop_assert_eq!(result1, result2,
            "Reward calculation non-deterministic: {} vs {}", result1, result2);
    }

    /// Property: Total staked equals sum of individual stakes
    #[test]
    fn prop_total_staked_consistency(
        stake_amounts in proptest::collection::vec(10i128..=1_000i128, 1..=5),
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let (client, stake_token, _, _) = setup_staking(&env);

        let mut expected_total = 0i128;
        let mut stakers = vec![];

        for amount in stake_amounts {
            let staker = Address::generate(&env);
            StellarAssetClient::new(&env, &stake_token).mint(&staker, &amount);

            if client.try_stake(&staker, &amount).is_ok() {
                expected_total += amount;
                stakers.push((staker, amount));
            }
        }

        let reported_total = client.get_total_staked();

        let mut individual_sum = 0i128;
        for (staker, _) in &stakers {
            individual_sum += client.get_stake(staker.clone());
        }

        prop_assert_eq!(reported_total, expected_total,
            "Total staked mismatch: reported={}, expected={}", reported_total, expected_total);
        prop_assert_eq!(individual_sum, reported_total,
            "Sum of individual stakes != total: sum={}, total={}", individual_sum, reported_total);
    }

    /// Property: Rewards never go negative
    #[test]
    fn prop_rewards_never_negative(
        stake_amount in 100i128..=1_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let (client, stake_token, _, _) = setup_staking(&env);
        let staker = Address::generate(&env);

        StellarAssetClient::new(&env, &stake_token).mint(&staker, &stake_amount);
        let _ = client.try_stake(&staker, &stake_amount);

        // Try claiming before any rewards added
        let _ = client.try_claim_rewards(&staker);

        let rewards = client.get_rewards(staker);
        prop_assert!(rewards >= 0, "Rewards went negative: {}", rewards);
    }
}
