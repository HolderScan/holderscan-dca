use anchor_lang::prelude::*;

/// Hard ceiling on the percentage fee. Bounds admin authority so a compromised
/// admin key cannot front-run `create_order` with a confiscatory fee change.
/// 300 bps = 3%, which leaves headroom above the intended 45 bps (0.45%).
pub const MAX_FEE_BPS: u16 = 300;

/// Hard ceiling on the absolute fee floor. Same rationale as MAX_FEE_BPS —
/// a compromised admin shouldn't be able to set the floor to a confiscatory
/// level. 1 SOL is 100x the intended 0.01 SOL floor.
pub const MAX_MIN_FEE_LAMPORTS: u64 = 1_000_000_000;

/// Bounds on schedule parameters. Primarily defend against fat-finger typos in
/// admin proposals; a compromised admin key can still pick any value in range.
pub const MIN_CYCLE_FREQUENCY: i64 = 60;
pub const MAX_CYCLE_FREQUENCY: i64 = 30 * 24 * 60 * 60; // 30 days
pub const MAX_NUM_CYCLES: u64 = 1_000;

/// Tolerance for user-supplied `created_at` vs on-chain clock. Accommodates
/// wall-clock drift between the signer and the validator without letting
/// callers seed order PDAs with arbitrary timestamps.
pub const CREATED_AT_TOLERANCE_SECS: i64 = 60;

#[account]
#[derive(InitSpace)]
pub struct DcaConfig {
    pub admin: Pubkey,           // who can update config
    pub pending_admin: Option<Pubkey>, // proposed new admin (two-step transfer)
    pub fee_vault: Pubkey,       // where fees accumulate
    pub keeper: Pubkey,          // authorized cycle executor
    /// Percentage fee on the user's input amount, in basis points (e.g. 45 = 0.45%).
    /// Fee is taken out of the input — the user signs for one number and the DCA
    /// schedule is funded with the remainder.
    pub fee_bps: u16,
    /// Absolute fee floor in wSOL lamports. Fee charged is max(input*bps/10000, this).
    pub min_fee_lamports: u64,
    pub default_cycle_frequency: i64, // seconds between cycles (default: 14400 = 4h)
    pub default_num_cycles: u64,      // number of cycles (default: 42 = 7 days @ 4h)
    /// Minimum input amount (in wSOL lamports) required to create an order.
    /// Gates unprofitably-small orders. Applied to the gross input — the value
    /// the user types in — not the post-fee notional. Admin-tunable.
    pub min_total_in_amount: u64,
    pub paused: bool,
    pub bump: u8,
}

impl DcaConfig {
    /// Compute the upfront fee on the user's input amount: the greater of the
    /// percentage fee and the absolute floor. wSOL-only at the call site, so
    /// `total_in_amount` and the returned value are both in lamports.
    pub fn compute_fee(&self, total_in_amount: u64) -> Option<u64> {
        let pct_fee = (total_in_amount as u128)
            .checked_mul(self.fee_bps as u128)?
            .checked_div(10_000)? as u64;
        Some(pct_fee.max(self.min_fee_lamports))
    }
}
