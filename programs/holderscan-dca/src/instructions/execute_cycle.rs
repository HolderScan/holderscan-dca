use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount, Transfer};
use crate::state::{DcaConfig, DcaOrder};
use crate::errors::DcaError;
use crate::events::CycleExecuted;

// Keeper contract (not enforceable on-chain — do not loosen without a plan):
//   1. The keeper MUST bundle this instruction + the Jupiter swap ix(s) + the
//      owner-delivery SPL transfer into a SINGLE transaction. If the swap or
//      delivery fails, the whole tx must revert so the cycle isn't consumed.
//      Cycle debit and swap/delivery are otherwise non-atomic — a split-tx
//      design leaves tokens stranded in the keeper ATA on any partial failure.
//   2. Keeper scheduling MUST be monotonic — poll exactly on the cycle boundary
//      with non-negative jitter only. Polling before `next_cycle_at` fails
//      `CycleTooEarly`; the keeper is the only thing enforcing the upper bound.

#[derive(Accounts)]
pub struct ExecuteCycle<'info> {
    #[account(
        constraint = keeper.key() == config.keeper @ DcaError::UnauthorizedKeeper,
    )]
    pub keeper: Signer<'info>,

    #[account(
        seeds = [b"dca_config"],
        bump = config.bump,
        constraint = !config.paused @ DcaError::ProgramPaused,
    )]
    pub config: Account<'info, DcaConfig>,

    #[account(
        mut,
        has_one = owner,
        constraint = order.is_active @ DcaError::OrderInactive,
    )]
    pub order: Account<'info, DcaOrder>,

    /// Order owner — receives residual tokens and rent refund when the order completes
    /// CHECK: validated via has_one on order
    #[account(mut)]
    pub owner: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [b"escrow", order.key().as_ref()],
        bump = order.escrow_bump,
        constraint = escrow_token_account.mint == order.input_mint,
    )]
    pub escrow_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: PDA authority over escrow
    #[account(
        seeds = [b"escrow_auth", order.key().as_ref()],
        bump,
    )]
    pub escrow_authority: UncheckedAccount<'info>,

    /// Owner's input ATA — receives any residual escrow balance on the final cycle
    #[account(
        mut,
        constraint = user_input_ata.mint == order.input_mint,
        constraint = user_input_ata.owner == order.owner,
    )]
    pub user_input_ata: Box<Account<'info, TokenAccount>>,

    /// Keeper's ATA for the input mint — receives tokens to swap via Jupiter
    #[account(
        mut,
        constraint = keeper_input_ata.mint == order.input_mint,
        constraint = keeper_input_ata.owner == keeper.key(),
    )]
    pub keeper_input_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<ExecuteCycle>) -> Result<()> {
    let order = &ctx.accounts.order;
    let clock = Clock::get()?;

    // Time gate
    require!(
        clock.unix_timestamp >= order.next_cycle_at,
        DcaError::CycleTooEarly
    );
    require!(order.cycles_remaining > 0, DcaError::OrderComplete);

    let swap_amount = order.in_amount_per_cycle;

    // Build PDA signer seeds
    let order_key = ctx.accounts.order.key();
    let seeds: &[&[u8]] = &[
        b"escrow_auth",
        order_key.as_ref(),
        &[ctx.bumps.escrow_authority],
    ];
    let signer_seeds = &[seeds];

    // Transfer cycle amount to keeper (fee was collected upfront at order creation)
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.escrow_token_account.to_account_info(),
                to: ctx.accounts.keeper_input_ata.to_account_info(),
                authority: ctx.accounts.escrow_authority.to_account_info(),
            },
            signer_seeds,
        ),
        swap_amount,
    )?;

    // Update state
    let order = &mut ctx.accounts.order;
    order.cycles_remaining = order
        .cycles_remaining
        .checked_sub(1)
        .ok_or(DcaError::MathOverflow)?;
    order.next_cycle_at = order
        .next_cycle_at
        .checked_add(order.cycle_frequency)
        .ok_or(DcaError::MathOverflow)?;

    let is_final = order.cycles_remaining == 0;
    if is_final {
        order.is_active = false;
    }

    emit!(CycleExecuted {
        order: order_key,
        cycles_remaining: order.cycles_remaining,
        swap_amount,
        timestamp: clock.unix_timestamp,
    });

    // On the final cycle, drain any residual escrow balance back to the owner
    // and close both the escrow token account and the order account so rent is returned.
    if is_final {
        ctx.accounts.escrow_token_account.reload()?;
        let residual = ctx.accounts.escrow_token_account.amount;
        if residual > 0 {
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
                residual,
            )?;
        }

        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            CloseAccount {
                account: ctx.accounts.escrow_token_account.to_account_info(),
                destination: ctx.accounts.owner.to_account_info(),
                authority: ctx.accounts.escrow_authority.to_account_info(),
            },
            signer_seeds,
        ))?;

        ctx.accounts.order.close(ctx.accounts.owner.to_account_info())?;
    }

    Ok(())
}