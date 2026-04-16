use anchor_lang::prelude::*;
use crate::state::DcaConfig;
use crate::errors::DcaError;

#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    pub new_admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"dca_config"],
        bump = config.bump,
        constraint = config.pending_admin.is_some() @ DcaError::NoPendingAdmin,
        constraint = config.pending_admin.unwrap() == new_admin.key() @ DcaError::PendingAdminMismatch,
    )]
    pub config: Account<'info, DcaConfig>,
}

pub fn handler(ctx: Context<AcceptAdmin>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.admin = ctx.accounts.new_admin.key();
    config.pending_admin = None;
    Ok(())
}
