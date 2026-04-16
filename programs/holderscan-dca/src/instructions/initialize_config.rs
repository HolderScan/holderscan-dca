use anchor_lang::prelude::*;
use crate::state::{
    DcaConfig, FeeTiers, MAX_CYCLE_FREQUENCY, MAX_FEE_BPS, MAX_NUM_CYCLES, MIN_CYCLE_FREQUENCY,
};
use crate::errors::DcaError;

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        init,
        payer = admin,
        space = 8 + DcaConfig::INIT_SPACE,
        seeds = [b"dca_config"],
        bump,
    )]
    pub config: Account<'info, DcaConfig>,

    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<InitializeConfig>,
    fee_vault: Pubkey,
    keeper: Pubkey,
    fee_tiers: FeeTiers,
    default_cycle_frequency: i64,
    default_num_cycles: u64,
    min_total_in_amount: u64,
) -> Result<()> {
    validate_fee_tiers(&fee_tiers)?;
    validate_cycle_frequency(default_cycle_frequency)?;
    validate_num_cycles(default_num_cycles)?;
    require!(
        min_total_in_amount >= default_num_cycles,
        DcaError::MinTotalBelowNumCycles,
    );

    let config = &mut ctx.accounts.config;
    config.admin = ctx.accounts.admin.key();
    config.pending_admin = None;
    config.fee_vault = fee_vault;
    config.keeper = keeper;
    config.fee_tiers = fee_tiers;
    config.default_cycle_frequency = default_cycle_frequency;
    config.default_num_cycles = default_num_cycles;
    config.min_total_in_amount = min_total_in_amount;
    config.paused = false;
    config.bump = ctx.bumps.config;

    Ok(())
}

pub fn validate_fee_tiers(t: &FeeTiers) -> Result<()> {
    require!(t.tier_1_fee_bps <= MAX_FEE_BPS, DcaError::FeeTooHigh);
    require!(t.tier_2_fee_bps <= MAX_FEE_BPS, DcaError::FeeTooHigh);
    require!(t.tier_3_fee_bps <= MAX_FEE_BPS, DcaError::FeeTooHigh);
    require!(
        t.tier_1_threshold_lamports < t.tier_2_threshold_lamports,
        DcaError::InvalidFeeTiers
    );
    Ok(())
}

pub fn validate_cycle_frequency(frequency: i64) -> Result<()> {
    require!(frequency >= MIN_CYCLE_FREQUENCY, DcaError::FrequencyTooLow);
    require!(frequency <= MAX_CYCLE_FREQUENCY, DcaError::FrequencyTooHigh);
    Ok(())
}

pub fn validate_num_cycles(num_cycles: u64) -> Result<()> {
    require!(num_cycles > 0, DcaError::InvalidAmount);
    require!(num_cycles <= MAX_NUM_CYCLES, DcaError::NumCyclesTooHigh);
    Ok(())
}
