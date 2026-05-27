//! Concentrated Liquidity AMM (Uniswap v3-style tick-based ranges).
//! Standalone contract — does NOT modify the existing AMM pool.
#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env};
use soroban_sdk::token::Client as TokenClient;

const PRICE_SCALE: i128 = 1_000_000;
const TICK_BASE_NUM: i128 = 1_000_100;
const TICK_BASE_DEN: i128 = PRICE_SCALE;
const MIN_TICK: i32 = -887_272;
const MAX_TICK: i32 = 887_272;

#[contracttype]
pub enum DataKey {
    TokenA, TokenB, FeeBps, CurrentTick,
    FeeGrowthGlobalA, FeeGrowthGlobalB, ActiveLiquidity,
    Position(Address, i32, i32),
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Position {
    pub lower_tick: i32,
    pub upper_tick: i32,
    pub liquidity: i128,
    pub fee_growth_inside_a: i128,
    pub fee_growth_inside_b: i128,
    pub tokens_owed: (i128, i128),
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PoolState {
    pub sqrt_price: u128,
    pub current_tick: i32,
    pub active_liquidity: i128,
}

#[contract]
pub struct ConcentratedLiquidity;


#[contractimpl]
impl ConcentratedLiquidity {
    /// One-time initialisation. Sets token pair, fee, and starting tick.
    pub fn initialize(env: Env, token_a: Address, token_b: Address, fee_bps: i128, initial_tick: i32) {
        assert!(!env.storage().instance().has(&DataKey::TokenA), "already initialized");
        assert!(token_a != token_b, "tokens must differ");
        assert!((0..=10_000).contains(&fee_bps), "invalid fee_bps");
        assert!(initial_tick >= MIN_TICK && initial_tick <= MAX_TICK, "tick out of range");
        env.storage().instance().set(&DataKey::TokenA, &token_a);
        env.storage().instance().set(&DataKey::TokenB, &token_b);
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
        env.storage().instance().set(&DataKey::CurrentTick, &initial_tick);
        env.storage().instance().set(&DataKey::FeeGrowthGlobalA, &0_i128);
        env.storage().instance().set(&DataKey::FeeGrowthGlobalB, &0_i128);
        env.storage().instance().set(&DataKey::ActiveLiquidity, &0_i128);
    }    pub fn mint_position(env: Env, provider: Address, lower_tick: i32, upper_tick: i32, amount_a_desired: i128, amount_b_desired: i128, min_a: i128, min_b: i128) -> (i128, i128) {
        provider.require_auth();
        assert!(lower_tick < upper_tick, "lower_tick must be < upper_tick");
        assert!(lower_tick >= MIN_TICK && upper_tick <= MAX_TICK, "tick out of range");
        assert!(amount_a_desired > 0 || amount_b_desired > 0, "zero amounts");
        let current_tick: i32 = env.storage().instance().get(&DataKey::CurrentTick).unwrap();
        let token_a: Address = env.storage().instance().get(&DataKey::TokenA).unwrap();
        let token_b: Address = env.storage().instance().get(&DataKey::TokenB).unwrap();
        let (amount_a, amount_b) = Self::amounts_for_liquidity(current_tick, lower_tick, upper_tick, amount_a_desired, amount_b_desired);
        assert!(amount_a >= min_a, "slippage: amount_a too low");
        assert!(amount_b >= min_b, "slippage: amount_b too low");
        let liquidity = Self::liquidity_from_amounts(current_tick, lower_tick, upper_tick, amount_a, amount_b);
        assert!(liquidity > 0, "liquidity would be zero");
        if amount_a > 0 { TokenClient::new(&env, &token_a).transfer(&provider, &env.current_contract_address(), &amount_a); }
        if amount_b > 0 { TokenClient::new(&env, &token_b).transfer(&provider, &env.current_contract_address(), &amount_b); }
        let pos_key = DataKey::Position(provider.clone(), lower_tick, upper_tick);
        let fg_a: i128 = env.storage().instance().get(&DataKey::FeeGrowthGlobalA).unwrap_or(0);
        let fg_b: i128 = env.storage().instance().get(&DataKey::FeeGrowthGlobalB).unwrap_or(0);
        let mut pos: Position = env.storage().instance().get(&pos_key).unwrap_or(Position { lower_tick, upper_tick, liquidity: 0, fee_growth_inside_a: fg_a, fee_growth_inside_b: fg_b, tokens_owed: (0, 0) });
        let (oa, ob) = Self::pending_fees(&pos, fg_a, fg_b);
        pos.tokens_owed = (pos.tokens_owed.0 + oa, pos.tokens_owed.1 + ob);
        pos.fee_growth_inside_a = fg_a; pos.fee_growth_inside_b = fg_b;
        pos.liquidity += liquidity;
        env.storage().instance().set(&pos_key, &pos);
        if current_tick >= lower_tick && current_tick < upper_tick {
            let active: i128 = env.storage().instance().get(&DataKey::ActiveLiquidity).unwrap_or(0);
            env.storage().instance().set(&DataKey::ActiveLiquidity, &(active + liquidity));
        }
        env.events().publish((symbol_short!("mint_pos"), provider), (lower_tick, upper_tick, liquidity, amount_a, amount_b));
        (amount_a, amount_b)
    }
    pub fn burn_position(env: Env, provider: Address, lower_tick: i32, upper_tick: i32, liquidity: i128) -> (i128, i128) {
        provider.require_auth();
        assert!(liquidity > 0, "liquidity must be positive");
        let pos_key = DataKey::Position(provider.clone(), lower_tick, upper_tick);
        let mut pos: Position = env.storage().instance().get(&pos_key).expect("position not found");
        assert!(pos.liquidity >= liquidity, "insufficient liquidity");
        let current_tick: i32 = env.storage().instance().get(&DataKey::CurrentTick).unwrap();
        let token_a: Address = env.storage().instance().get(&DataKey::TokenA).unwrap();
        let token_b: Address = env.storage().instance().get(&DataKey::TokenB).unwrap();
        let fg_a: i128 = env.storage().instance().get(&DataKey::FeeGrowthGlobalA).unwrap_or(0);
        let fg_b: i128 = env.storage().instance().get(&DataKey::FeeGrowthGlobalB).unwrap_or(0);
        let (oa, ob) = Self::pending_fees(&pos, fg_a, fg_b);
        pos.tokens_owed = (pos.tokens_owed.0 + oa, pos.tokens_owed.1 + ob);
        pos.fee_growth_inside_a = fg_a; pos.fee_growth_inside_b = fg_b;
        let (amount_a, amount_b) = Self::amounts_for_liquidity(current_tick, lower_tick, upper_tick, liquidity, liquidity);
        pos.liquidity -= liquidity;
        env.storage().instance().set(&pos_key, &pos);
        if current_tick >= lower_tick && current_tick < upper_tick {
            let active: i128 = env.storage().instance().get(&DataKey::ActiveLiquidity).unwrap_or(0);
            env.storage().instance().set(&DataKey::ActiveLiquidity, &(if active > liquidity { active - liquidity } else { 0 }));
        }
        if amount_a > 0 { TokenClient::new(&env, &token_a).transfer(&env.current_contract_address(), &provider, &amount_a); }
        if amount_b > 0 { TokenClient::new(&env, &token_b).transfer(&env.current_contract_address(), &provider, &amount_b); }
        env.events().publish((symbol_short!("burn_pos"), provider), (lower_tick, upper_tick, liquidity, amount_a, amount_b));
        (amount_a, amount_b)
    }
    pub fn collect_fees(env: Env, provider: Address, lower_tick: i32, upper_tick: i32) -> (i128, i128) {
        provider.require_auth();
        let pos_key = DataKey::Position(provider.clone(), lower_tick, upper_tick);
        let mut pos: Position = env.storage().instance().get(&pos_key).expect("position not found");
        let fg_a: i128 = env.storage().instance().get(&DataKey::FeeGrowthGlobalA).unwrap_or(0);
        let fg_b: i128 = env.storage().instance().get(&DataKey::FeeGrowthGlobalB).unwrap_or(0);
        let (na, nb) = Self::pending_fees(&pos, fg_a, fg_b);
        let total_a = pos.tokens_owed.0 + na; let total_b = pos.tokens_owed.1 + nb;
        pos.tokens_owed = (0, 0); pos.fee_growth_inside_a = fg_a; pos.fee_growth_inside_b = fg_b;
        env.storage().instance().set(&pos_key, &pos);
        let token_a: Address = env.storage().instance().get(&DataKey::TokenA).unwrap();
        let token_b: Address = env.storage().instance().get(&DataKey::TokenB).unwrap();
        if total_a > 0 { TokenClient::new(&env, &token_a).transfer(&env.current_contract_address(), &provider, &total_a); }
        if total_b > 0 { TokenClient::new(&env, &token_b).transfer(&env.current_contract_address(), &provider, &total_b); }
        (total_a, total_b)
    }
    pub fn get_position(env: Env, provider: Address, lower_tick: i32, upper_tick: i32) -> Position {
        env.storage().instance().get(&DataKey::Position(provider, lower_tick, upper_tick)).expect("position not found")
    }
    pub fn current_tick(env: Env) -> i32 { env.storage().instance().get(&DataKey::CurrentTick).unwrap() }
    pub fn active_liquidity(env: Env) -> i128 { env.storage().instance().get(&DataKey::ActiveLiquidity).unwrap_or(0) }
    pub fn get_pool_state(env: Env) -> PoolState {
        let current_tick: i32 = env.storage().instance().get(&DataKey::CurrentTick).unwrap_or(0);
        let active_liquidity: i128 = env.storage().instance().get(&DataKey::ActiveLiquidity).unwrap_or(0);
        let price = Self::tick_to_price(current_tick);
        let sqrt_p = Self::sqrt(price);
        let sqrt_price = (sqrt_p as u128) * (1u128 << 96) / 1000u128;
        PoolState {
            sqrt_price,
            current_tick,
            active_liquidity,
        }
    }
    pub fn swap(env: Env, buyer: Address, token_in: Address, amount_in: i128, target_tick: i32) -> i128 {
        buyer.require_auth();
        assert!(amount_in > 0, "amount_in must be positive");
        let token_a: Address = env.storage().instance().get(&DataKey::TokenA).unwrap();
        let token_b: Address = env.storage().instance().get(&DataKey::TokenB).unwrap();
        assert!(token_in == token_a || token_in == token_b, "invalid token_in");
        assert!(target_tick >= MIN_TICK && target_tick <= MAX_TICK, "target tick out of range");
        env.storage().instance().set(&DataKey::CurrentTick, &target_tick);
        TokenClient::new(&env, &token_in).transfer(&buyer, &env.current_contract_address(), &amount_in);
        let token_out = if token_in == token_a { token_b } else { token_a };
        TokenClient::new(&env, &token_out).transfer(&env.current_contract_address(), &buyer, &amount_in);
        env.events().publish((soroban_sdk::symbol_short!("swap"), buyer), (token_in, amount_in, target_tick));
        amount_in
    }
    fn sqrt(y: i128) -> i128 {
        if y > 3 {
            let mut z = y;
            let mut x = y / 2 + 1;
            while x < z {
                z = x;
                x = (y / x + x) / 2;
            }
            z
        } else if y != 0 {
            1
        } else {
            0
        }
    }

    pub fn tick_to_price(tick: i32) -> i128 {
        if tick == 0 { return 1_000_000; }
        let abs_tick = tick.unsigned_abs() as i128;
        let iters = abs_tick.min(300);
        let mut price = 1_000_000_i128;
        for _ in 0..iters { price = price * 1_000_100 / 1_000_000; }
        if tick < 0 { price = 1_000_000 * 1_000_000 / price; }
        price
    }
    fn amounts_for_liquidity(ct: i32, lt: i32, ut: i32, ad: i128, bd: i128) -> (i128, i128) {
        if ct < lt { (ad, 0) } else if ct >= ut { (0, bd) } else {
            let pl = Self::tick_to_price(lt); let pu = Self::tick_to_price(ut); let pc = Self::tick_to_price(ct);
            let range = pu - pl; if range == 0 { return (ad/2, bd/2); }
            let below = pc - pl;
            (ad * (range - below) / range, bd * below / range)
        }
    }
    fn liquidity_from_amounts(ct: i32, lt: i32, ut: i32, a: i128, b: i128) -> i128 {
        if ct < lt { a } else if ct >= ut { b } else { a.min(b).max(1) }
    }
    fn pending_fees(pos: &Position, fg_a: i128, fg_b: i128) -> (i128, i128) {
        let da = fg_a - pos.fee_growth_inside_a; let db = fg_b - pos.fee_growth_inside_b;
        let oa = if da > 0 { pos.liquidity * da / 1_000_000 } else { 0 };
        let ob = if db > 0 { pos.liquidity * db / 1_000_000 } else { 0 };
        (oa, ob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[test]
    fn test_pool_state_flow() {
        let env = Env::default();
        env.budget().reset_unlimited();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let provider = Address::generate(&env);

        let token_a = env.register_stellar_asset_contract_v2(admin.clone()).address();
        let token_b = env.register_stellar_asset_contract_v2(admin.clone()).address();

        let cl_addr = env.register_contract(None, ConcentratedLiquidity);
        let client = ConcentratedLiquidityClient::new(&env, &cl_addr);

        // 1. Test after initialize
        client.initialize(&token_a, &token_b, &30_i128, &0_i32);
        let state1 = client.get_pool_state();
        assert_eq!(state1.current_tick, 0);
        assert_eq!(state1.active_liquidity, 0);
        // Price for tick 0 is 1.0 (2^96)
        assert_eq!(state1.sqrt_price, 1u128 << 96);

        // Mint setup: mint tokens to provider
        let sac_a = soroban_sdk::token::StellarAssetClient::new(&env, &token_a);
        let sac_b = soroban_sdk::token::StellarAssetClient::new(&env, &token_b);
        sac_a.mint(&provider, &1_000_000_i128);
        sac_b.mint(&provider, &1_000_000_i128);

        // 2. Test after mint_position
        // Range [-100, 100] covers current_tick = 0, so active_liquidity should increase.
        client.mint_position(&provider, &-100_i32, &100_i32, &10_000_i128, &10_000_i128, &0_i128, &0_i128);
        let state2 = client.get_pool_state();
        assert_eq!(state2.current_tick, 0);
        assert!(state2.active_liquidity > 0);

        // Mint token_b to contract to support swap
        sac_b.mint(&cl_addr, &10_000_i128);

        // 3. Test after a swap (simulated by updating tick to 50)
        client.swap(&provider, &token_a, &1_000_i128, &50_i32);
        let state3 = client.get_pool_state();
        assert_eq!(state3.current_tick, 50);
        // Ensure price is calculated correctly for tick 50
        assert!(state3.sqrt_price > (1u128 << 96));
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events as _};
    use soroban_sdk::token::StellarAssetClient;
    use soroban_sdk::{IntoVal, Val, Vec as SdkVec};

    // #157: burn_position must emit ("burn_pos", provider) with
    // (lower_tick, upper_tick, liquidity, amount_a, amount_b) — and the amounts
    // must match what was actually returned to the provider.
    #[test]
    fn burn_position_emits_burn_pos_event_with_returned_amounts() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let token_a = env.register_stellar_asset_contract_v2(admin.clone());
        let token_b = env.register_stellar_asset_contract_v2(admin.clone());
        let token_a_addr = token_a.address();
        let token_b_addr = token_b.address();

        let contract_id = env.register_contract(None, ConcentratedLiquidity);
        let client = ConcentratedLiquidityClient::new(&env, &contract_id);
        // current_tick = 0; a range entirely above it is pure token-A, so mint
        // and burn amounts are deterministic (liquidity == amount_a, and the
        // burn returns exactly what was deposited).
        client.initialize(&token_a_addr, &token_b_addr, &30_i128, &0_i32);

        let provider = Address::generate(&env);
        StellarAssetClient::new(&env, &token_a_addr).mint(&provider, &1_000_i128);

        let lower_tick = 100_i32;
        let upper_tick = 200_i32;
        let (mint_a, mint_b) = client.mint_position(
            &provider, &lower_tick, &upper_tick, &1_000_i128, &0_i128, &0_i128, &0_i128,
        );
        assert_eq!((mint_a, mint_b), (1_000_i128, 0_i128));

        let liquidity = client.get_position(&provider, &lower_tick, &upper_tick).liquidity;
        let (amount_a, amount_b) =
            client.burn_position(&provider, &lower_tick, &upper_tick, &liquidity);
        assert_eq!((amount_a, amount_b), (1_000_i128, 0_i128));

        // mint_pos and burn_pos share the same data shape, so match on the topic.
        let expected_topics: SdkVec<Val> =
            (symbol_short!("burn_pos"), provider.clone()).into_val(&env);
        let event = env
            .events()
            .all()
            .iter()
            .find(|e| e.0 == contract_id && e.1 == expected_topics)
            .expect("burn_pos event not emitted");

        let data: (i32, i32, i128, i128, i128) = event.2.into_val(&env);
        assert_eq!(data, (lower_tick, upper_tick, liquidity, amount_a, amount_b));
    }
}
