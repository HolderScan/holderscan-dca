use anchor_lang::prelude::*;

/// Hard ceiling on any single fee tier. Bounds admin authority so a compromised
/// admin key cannot front-run `create_order` with a confiscatory fee change.
/// 300 bps = 3%, which leaves headroom above the intended 50-100 bps product range.
pub const MAX_FEE_BPS: u16 = 300;

/// Bounds on schedule parameters. Primarily defend against fat-finger typos in
/// admin proposals; a compromised admin key can still pick any value in range.
pub const MIN_CYCLE_FREQUENCY: i64 = 60;
pub const MAX_CYCLE_FREQUENCY: i64 = 30 * 24 * 60 * 60; // 30 days
pub const MAX_NUM_CYCLES: u64 = 1_000;

/// Tolerance for user-supplied `created_at` vs on-chain clock. Accommodates
/// wall-clock drift between the signer and the validator without letting
/// callers seed order PDAs with arbitrary timestamps.
pub const CREATED_AT_TOLERANCE_SECS: i64 = 60;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace)]
pub struct FeeTiers {
    /// Orders with notional < tier_1_threshold_lamports pay tier_1_fee_bps.
    /// Orders with notional in [tier_1_threshold, tier_2_threshold) pay tier_2_fee_bps.
    /// Orders with notional >= tier_2_threshold pay tier_3_fee_bps.
    pub tier_1_fee_bps: u16,
    pub tier_2_fee_bps: u16,
    pub tier_3_fee_bps: u16,
    pub tier_1_threshold_lamports: u64,
    pub tier_2_threshold_lamports: u64,
}

impl FeeTiers {
    pub fn fee_bps_for(&self, total_in_amount: u64) -> u16 {
        if total_in_amount < self.tier_1_threshold_lamports {
            self.tier_1_fee_bps
        } else if total_in_amount < self.tier_2_threshold_lamports {
            self.tier_2_fee_bps
        } else {
            self.tier_3_fee_bps
        }
    }
}

#[account]
#[derive(InitSpace)]
pub struct DcaConfig {
    pub admin: Pubkey,           // who can update config
    pub pending_admin: Option<Pubkey>, // proposed new admin (two-step transfer)
    pub fee_vault: Pubkey,       // where fees accumulate
    pub keeper: Pubkey,          // authorized cycle executor
    pub fee_tiers: FeeTiers,     // volume-tiered fee schedule (wSOL notional)
    pub default_cycle_frequency: i64, // seconds between cycles (default: 14400 = 4h)
    pub default_num_cycles: u64,      // number of cycles (default: 42 = 7 days @ 4h)
    /// Minimum total DCA notional (in wSOL lamports) required to create an
    /// order. Gates unprofitably-small orders. Admin-tunable via update_config.
    pub min_total_in_amount: u64,
    pub paused: bool,
    pub bump: u8,
}
