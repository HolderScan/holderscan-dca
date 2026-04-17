use anchor_lang::prelude::*;
use crate::state::DcaConfig;
use crate::errors::DcaError;
use crate::instructions::initialize_config::{
    validate_cycle_frequency, validate_fee_params, validate_num_cycles,
};

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(
        constraint = admin.key() == config.admin @ DcaError::UnauthorizedAdmin,
    )]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"dca_config"],
        bump = config.bump,
    )]
    pub config: Account<'info, DcaConfig>,
}

#[allow(clippy::too_many_arguments)]
pub fn handler(
    ctx: Context<UpdateConfig>,
    new_keeper: Option<Pubkey>,
    new_fee_vault: Option<Pubkey>,
    new_fee_bps: Option<u16>,
    new_min_fee_lamports: Option<u64>,
    new_default_cycle_frequency: Option<i64>,
    new_default_num_cycles: Option<u64>,
    new_min_total_in_amount: Option<u64>,
    paused: Option<bool>,
) -> Result<()> {
    let config = &mut ctx.accounts.config;

    if let Some(keeper) = new_keeper {
        config.keeper = keeper;
    }

    if let Some(fee_vault) = new_fee_vault {
        config.fee_vault = fee_vault;
    }

    if new_fee_bps.is_some() || new_min_fee_lamports.is_some() {
        let fee_bps = new_fee_bps.unwrap_or(config.fee_bps);
        let min_fee_lamports = new_min_fee_lamports.unwrap_or(config.min_fee_lamports);
        validate_fee_params(fee_bps, min_fee_lamports)?;
        config.fee_bps = fee_bps;
        config.min_fee_lamports = min_fee_lamports;
    }

    if let Some(frequency) = new_default_cycle_frequency {
        validate_cycle_frequency(frequency)?;
        config.default_cycle_frequency = frequency;
    }

    if let Some(num_cycles) = new_default_num_cycles {
        validate_num_cycles(num_cycles)?;
        config.default_num_cycles = num_cycles;
    }

    if let Some(min_total) = new_min_total_in_amount {
        config.min_total_in_amount = min_total;
    }

    if let Some(paused) = paused {
        config.paused = paused;
    }

    // Final invariant: every valid `create_order` must be able to pass its
    // `in_amount_per_cycle > 0` check when `total_in_amount == min_total_in_amount`.
    // That requires min_total >= default_num_cycles regardless of which fields
    // this call touched.
    require!(
        config.min_total_in_amount >= config.default_num_cycles,
        DcaError::MinTotalBelowNumCycles,
    );

    Ok(())
}
