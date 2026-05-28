//! LP Staking and Rewards Contract
//!
//! Liquidity providers can stake their LP tokens to earn reward tokens.
//! Uses a rewards-per-share accumulator pattern (similar to SushiSwap's MasterChef)
//! for efficient O(1) reward calculation per claim.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, Symbol,
};

// SEP-41 token interface
use soroban_sdk::token::Client as SepTokenClient;

/// LP token and reward token client interface.
#[soroban_sdk::contractclient(name = "TokenClient")]
pub trait TokenInterface {
    fn transfer(env: Env, from: Address, to: Address, amount: i128);
    fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128);
    fn balance(env: Env, id: Address) -> i128;
    fn approve(env: Env, spender: Address, amount: i128, expiration_ledger: u32);
}

// ── Storage keys ───────────────────────────────────────────────────────────

#[contracttype]
pub enum DataKey {
    /// LP token address
    LpToken,
    /// Reward token address
    RewardToken,
    /// Admin address (can add rewards)
    Admin,
    /// Total LP tokens staked
    TotalStaked,
    /// Accumulated rewards per LP token (scaled by 1e18)
    AccumulatedRewardsPerShare,
    /// Staker info: staked amount
    StakerAmount(Address),
    /// Staker info: rewards debt (to track already-distributed rewards)
    StakerRewardsDebt(Address),
    /// Remaining reward tokens available in pool
    RewardPoolBalance,
}

// ── Data structures ───────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub struct StakerInfo {
    pub staked_amount: i128,
    pub rewards_debt: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolInfo {
    pub lp_token: Address,
    pub reward_token: Address,
    pub admin: Address,
    pub total_staked: i128,
    pub reward_pool_balance: i128,
    pub accumulated_rewards_per_share: i128,
}

// ── Constants ──────────────────────────────────────────────────────────────

const SCALE_FACTOR: i128 = 1_000_000_000_000_000_000; // 1e18

// ── Contract ────────────────────────────────────────────────────────────────

#[contract]
pub struct Staking;

#[contractimpl]
impl Staking {
    /// Initialize the staking contract.
    ///
    /// # Parameters
    /// - `lp_token` – Address of the LP token to stake
    /// - `reward_token` – Address of the reward token to distribute
    /// - `admin` – Address authorized to add rewards
    pub fn initialize(env: Env, lp_token: Address, reward_token: Address, admin: Address) {
        assert!(
            !env.storage().instance().has(&DataKey::LpToken),
            "already initialized"
        );
        env.storage().instance().set(&DataKey::LpToken, &lp_token);
        env.storage()
            .instance()
            .set(&DataKey::RewardToken, &reward_token);
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::TotalStaked, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::AccumulatedRewardsPerShare, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::RewardPoolBalance, &0i128);
    }

    /// Add rewards to the pool. Admin only.
    ///
    /// # Parameters
    /// - `admin` – Must be the configured admin address
    /// - `amount` – Amount of reward tokens to add
    pub fn add_rewards(env: Env, admin: Address, amount: i128) {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        assert!(admin == stored_admin, "not admin");
        assert!(amount > 0, "amount must be positive");

        let reward_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::RewardToken)
            .unwrap();
        let pool_addr = env.current_contract_address();

        // Transfer reward tokens from admin to pool
        SepTokenClient::new(&env, &reward_token)
            .transfer_from(&admin, &admin, &pool_addr, &amount);

        // Update pool balance
        let current_balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::RewardPoolBalance)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::RewardPoolBalance, &(current_balance + amount));

        env.events()
            .publish((Symbol::new(&env, "rewards_added"),), (admin, amount));
    }

    /// Stake LP tokens to start earning rewards.
    ///
    /// # Parameters
    /// - `staker` – Address staking LP tokens
    /// - `amount` – Amount of LP tokens to stake
    pub fn stake(env: Env, staker: Address, amount: i128) {
        staker.require_auth();
        assert!(amount > 0, "amount must be positive");

        let lp_token: Address = env.storage().instance().get(&DataKey::LpToken).unwrap();
        let pool_addr = env.current_contract_address();

        // Transfer LP tokens from staker to pool
        SepTokenClient::new(&env, &lp_token).transfer_from(&staker, &staker, &pool_addr, &amount);

        // Update staker's staked amount
        let current_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerAmount(staker.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::StakerAmount(staker.clone()), &(current_staked + amount));

        // Update total staked
        let total_staked: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap();
        env.storage()
            .instance()
            .set(&DataKey::TotalStaked, &(total_staked + amount));

        // Update rewards debt
        let acc_per_share: i128 = env
            .storage()
            .instance()
            .get(&DataKey::AccumulatedRewardsPerShare)
            .unwrap_or(0);
        let current_debt: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerRewardsDebt(staker.clone()))
            .unwrap_or(0);
        let new_debt = current_debt + (amount * acc_per_share / SCALE_FACTOR);
        env.storage()
            .persistent()
            .set(&DataKey::StakerRewardsDebt(staker.clone()), &new_debt);

        env.events()
            .publish((Symbol::new(&env, "staked"),), (staker, amount));
    }

    /// Claim accrued rewards without unstaking.
    ///
    /// # Parameters
    /// - `staker` – Address claiming rewards
    ///
    /// # Returns
    /// Amount of reward tokens transferred
    pub fn claim(env: Env, staker: Address) -> i128 {
        staker.require_auth();

        let pending = Self::pending_rewards(env.clone(), staker.clone());
        assert!(pending > 0, "no pending rewards");

        let reward_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::RewardToken)
            .unwrap();
        let pool_addr = env.current_contract_address();

        // Update rewards debt to reflect claimed rewards
        let staked_amount: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerAmount(staker.clone()))
            .unwrap_or(0);
        let acc_per_share: i128 = env
            .storage()
            .instance()
            .get(&DataKey::AccumulatedRewardsPerShare)
            .unwrap_or(0);
        let new_debt = staked_amount * acc_per_share / SCALE_FACTOR;
        env.storage()
            .persistent()
            .set(&DataKey::StakerRewardsDebt(staker.clone()), &new_debt);

        // Transfer reward tokens to staker
        SepTokenClient::new(&env, &reward_token).transfer(&pool_addr, &staker, &pending);

        // Update pool balance
        let pool_balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::RewardPoolBalance)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::RewardPoolBalance, &(pool_balance - pending));

        env.events()
            .publish((Symbol::new(&env, "claimed"),), (staker, pending));

        pending
    }

    /// Unstake LP tokens and claim pending rewards.
    ///
    /// # Parameters
    /// - `staker` – Address unstaking LP tokens
    /// - `amount` – Amount of LP tokens to unstake
    ///
    /// # Returns
    /// Tuple (lp_returned, rewards_claimed)
    pub fn unstake(env: Env, staker: Address, amount: i128) -> (i128, i128) {
        staker.require_auth();
        assert!(amount > 0, "amount must be positive");

        let staked_amount: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerAmount(staker.clone()))
            .unwrap_or(0);
        assert!(staked_amount >= amount, "insufficient staked amount");

        // Claim pending rewards first
        let rewards = Self::claim(env.clone(), staker.clone());

        let lp_token: Address = env.storage().instance().get(&DataKey::LpToken).unwrap();
        let pool_addr = env.current_contract_address();

        // Transfer LP tokens back to staker
        SepTokenClient::new(&env, &lp_token).transfer(&pool_addr, &staker, &amount);

        // Update staked amount
        env.storage()
            .persistent()
            .set(&DataKey::StakerAmount(staker.clone()), &(staked_amount - amount));

        // Update total staked
        let total_staked: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap();
        env.storage()
            .instance()
            .set(&DataKey::TotalStaked, &(total_staked - amount));

        // Adjust rewards debt
        let acc_per_share: i128 = env
            .storage()
            .instance()
            .get(&DataKey::AccumulatedRewardsPerShare)
            .unwrap_or(0);
        let current_debt: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerRewardsDebt(staker.clone()))
            .unwrap_or(0);
        let debt_reduction = amount * acc_per_share / SCALE_FACTOR;
        let new_debt = current_debt.saturating_sub(debt_reduction);
        env.storage()
            .persistent()
            .set(&DataKey::StakerRewardsDebt(staker.clone()), &new_debt);

        env.events()
            .publish((Symbol::new(&env, "unstaked"),), (staker, amount, rewards));

        (amount, rewards)
    }

    /// View pending rewards for a staker.
    ///
    /// # Parameters
    /// - `staker` – Address to check pending rewards for
    ///
    /// # Returns
    /// Amount of pending reward tokens
    pub fn pending_rewards(env: Env, staker: Address) -> i128 {
        let staked_amount: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerAmount(staker.clone()))
            .unwrap_or(0);

        if staked_amount == 0 {
            return 0;
        }

        let acc_per_share: i128 = env
            .storage()
            .instance()
            .get(&DataKey::AccumulatedRewardsPerShare)
            .unwrap_or(0);
        let rewards_debt: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerRewardsDebt(staker))
            .unwrap_or(0);

        let pending = staked_amount * acc_per_share / SCALE_FACTOR - rewards_debt;
        pending.max(0)
    }

    /// Get pool information.
    pub fn get_pool_info(env: Env) -> PoolInfo {
        PoolInfo {
            lp_token: env.storage().instance().get(&DataKey::LpToken).unwrap(),
            reward_token: env
                .storage()
                .instance()
                .get(&DataKey::RewardToken)
                .unwrap(),
            admin: env.storage().instance().get(&DataKey::Admin).unwrap(),
            total_staked: env
                .storage()
                .instance()
                .get(&DataKey::TotalStaked)
                .unwrap_or(0),
            reward_pool_balance: env
                .storage()
                .instance()
                .get(&DataKey::RewardPoolBalance)
                .unwrap_or(0),
            accumulated_rewards_per_share: env
                .storage()
                .instance()
                .get(&DataKey::AccumulatedRewardsPerShare)
                .unwrap_or(0),
        }
    }

    /// Manually update the accumulated rewards per share (if needed for manual reward distribution).
    /// This allows distributing rewards without requiring new LP deposits.
    ///
    /// # Parameters
    /// - `admin` – Must be the configured admin
    /// - `new_rewards` – Amount of new rewards to distribute
    pub fn update_rewards(env: Env, admin: Address, new_rewards: i128) {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        assert!(admin == stored_admin, "not admin");
        assert!(new_rewards > 0, "new_rewards must be positive");

        let total_staked: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        assert!(total_staked > 0, "no stakers");

        let acc_per_share: i128 = env
            .storage()
            .instance()
            .get(&DataKey::AccumulatedRewardsPerShare)
            .unwrap_or(0);
        let rewards_increase = new_rewards * SCALE_FACTOR / total_staked;
        let new_acc_per_share = acc_per_share + rewards_increase;
        env.storage()
            .instance()
            .set(&DataKey::AccumulatedRewardsPerShare, &new_acc_per_share);

        // Reduce reward pool balance
        let pool_balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::RewardPoolBalance)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::RewardPoolBalance, &(pool_balance - new_rewards));

        env.events()
            .publish((Symbol::new(&env, "rewards_updated"),), (new_rewards,));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token::{StellarAssetClient, TokenClient as StellarTokenClient},
        Address, Env,
    };

    fn create_sac<'a>(
        env: &'a Env,
        admin: &Address,
    ) -> (StellarTokenClient<'a>, StellarAssetClient<'a>) {
        let contract = env.register_stellar_asset_contract_v2(admin.clone());
        (
            StellarTokenClient::new(env, &contract.address()),
            StellarAssetClient::new(env, &contract.address()),
        )
    }

    #[test]
    fn test_stake_and_claim() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let staker = Address::generate(&env);
        let staking_addr = env.register_contract(None, Staking);

        let (lp_token, lp_sac) = create_sac(&env, &admin);
        let (reward_token, reward_sac) = create_sac(&env, &admin);

        let staking = StakingClient::new(&env, &staking_addr);

        staking.initialize(
            &lp_token.address,
            &reward_token.address,
            &admin,
        );

        // Mint LP tokens to staker
        lp_sac.mint(&staker, &1000_i128);

        // Mint reward tokens to admin
        reward_sac.mint(&admin, &500_i128);

        // Add rewards to pool
        staking.add_rewards(&admin, &500_i128);

        // Stake LP tokens
        staking.stake(&staker, &1000_i128);

        let pool_info = staking.get_pool_info();
        assert_eq!(pool_info.total_staked, 1000);

        // Manually update rewards (distributing 100 tokens)
        staking.update_rewards(&admin, &100_i128);

        let pending = staking.pending_rewards(&staker);
        assert_eq!(pending, 100);

        // Claim rewards
        let claimed = staking.claim(&staker);
        assert_eq!(claimed, 100);
    }

    #[test]
    fn test_unstake_and_claim() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let staker = Address::generate(&env);
        let staking_addr = env.register_contract(None, Staking);

        let (lp_token, lp_sac) = create_sac(&env, &admin);
        let (reward_token, reward_sac) = create_sac(&env, &admin);

        let staking = StakingClient::new(&env, &staking_addr);

        staking.initialize(
            &lp_token.address,
            &reward_token.address,
            &admin,
        );

        lp_sac.mint(&staker, &1000_i128);
        reward_sac.mint(&admin, &500_i128);

        staking.add_rewards(&admin, &500_i128);
        staking.stake(&staker, &1000_i128);
        staking.update_rewards(&admin, &100_i128);

        let (lp_returned, rewards_claimed) = staking.unstake(&staker, &500_i128);

        assert_eq!(lp_returned, 500);
        assert_eq!(rewards_claimed, 100);

        let remaining_staked: i128 = staking.get_pool_info().total_staked;
        assert_eq!(remaining_staked, 500);
    }
}
