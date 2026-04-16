use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, CloseAccount};
use crate::state::DcaOrder;
use crate::errors::DcaError;
use crate::events::OrderCancelled;

#[derive(Accounts)]
pub struct CancelOrder<'info> {
    #[account(
        mut,
        constraint = owner.key() == order.owner @ DcaError::OrderInactive,
    )]
    pub owner: Signer<'info>,

    #[account(
        mut,
        constraint = order.is_active @ DcaError::OrderInactive,
        close = owner,
    )]
    pub order: Account<'info, DcaOrder>,

    #[account(
        mut,
        seeds = [b"escrow", order.key().as_ref()],
        bump = order.escrow_bump,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// CHECK: PDA authority over escrow
    #[account(
        seeds = [b"escrow_auth", order.key().as_ref()],
        bump,
    )]
    pub escrow_authority: UncheckedAccount<'info>,

    /// User's token account to receive refund
    #[account(
        mut,
        constraint = user_input_ata.mint == order.input_mint,
        constraint = user_input_ata.owner == owner.key(),
    )]
    pub user_input_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<CancelOrder>) -> Result<()> {
    let remaining = ctx.accounts.escrow_token_account.amount;

    let order_key = ctx.accounts.order.key();
    let seeds: &[&[u8]] = &[
        b"escrow_auth",
        order_key.as_ref(),
        &[ctx.bumps.escrow_authority],
    ];
    let signer_seeds = &[seeds];

    // Transfer remaining tokens back to user
    if remaining > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.escrow_token_account.to_account_info(),
                    to: ctx.accounts.user_input_ata.to_account_info(),
                    authority: ctx.accounts.escrow_authority.to_account_info(),
                },
                signer_seeds,
            ),
            remaining,
        )?;
    }

    // Close the escrow token account, return rent to owner
    token::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        CloseAccount {
            account: ctx.accounts.escrow_token_account.to_account_info(),
            destination: ctx.accounts.owner.to_account_info(),
            authority: ctx.accounts.escrow_authority.to_account_info(),
        },
        signer_seeds,
    ))?;

    emit!(OrderCancelled {
        order: ctx.accounts.order.key(),
        owner: ctx.accounts.owner.key(),
        refunded_amount: remaining,
    });

    Ok(())
}