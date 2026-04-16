use anchor_lang::prelude::*;
use crate::state::DcaConfig;
use crate::errors::DcaError;

#[derive(Accounts)]
pub struct ProposeAdmin<'info> {
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

pub fn handler(ctx: Context<ProposeAdmin>, new_admin: Pubkey) -> Result<()> {
    ctx.accounts.config.pending_admin = Some(new_admin);
    Ok(())
}
