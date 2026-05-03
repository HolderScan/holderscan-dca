use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::errors::DcaError;
use crate::events::CycleRefunded;
use crate::state::{DcaConfig, DcaOrder};

/// Keeper-only rollback for when the off-chain swap stage fails after
/// `execute_cycle` has already drained the cycle's wSOL to the keeper's
/// input ATA. Returns the wSOL to the order's escrow PDA, re-credits the
/// cycle, and rewinds `next_cycle_at` so the next keeper poll retries the
/// cycle from a fully consistent state.
///
/// NOT callable on a closed order — a final-cycle failure cannot be unwound
/// by the program (the escrow and order accounts are gone); the keeper must
/// SPL-transfer the drained wSOL directly to the owner in that case.
#[derive(Accounts)]
pub struct RefundCycle<'info> {
    /// Keeper pays the tx fee and signs the SPL transfer (it owns the source ATA).
    #[account(
        mut,
        constraint = keeper.key() == config.keeper @ DcaError::UnauthorizedKeeper,
    )]
    pub keeper: Signer<'info>,

    #[account(
        seeds = [b"dca_config"],
        bump = config.bump,
    )]
    pub config: Account<'info, DcaConfig>,

    #[account(
        mut,
        has_one = owner,
        constraint = order.is_active @ DcaError::OrderInactive,
    )]
    pub order: Account<'info, DcaOrder>,

    /// CHECK: validated via `has_one = owner` on order
    pub owner: UncheckedAccount<'info>,

    /// Keeper's input-mint ATA holding the funds from the failed cycle.
    #[account(
        mut,
        constraint = keeper_input_ata.mint == order.input_mint,
        constraint = keeper_input_ata.owner == keeper.key(),
    )]
    pub keeper_input_ata: Box<Account<'info, TokenAccount>>,

    /// Escrow PDA — receives the rolled-back cycle's wSOL so the
    /// `escrow.amount ≥ cycles_remaining × in_amount_per_cycle` invariant
    /// is restored before the next tick.
    #[account(
        mut,
        seeds = [b"escrow", order.key().as_ref()],
        bump = order.escrow_bump,
        constraint = escrow_token_account.mint == order.input_mint,
    )]
    pub escrow_token_account: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<RefundCycle>) -> Result<()> {
    let order = &ctx.accounts.order;
    let refund_amount = order.in_amount_per_cycle;

    // Return the cycle's wSOL to the escrow so the next tick retries from a
    // consistent state — escrow balance and `cycles_remaining` stay in sync.
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.keeper_input_ata.to_account_info(),
                to: ctx.accounts.escrow_token_account.to_account_info(),
                authority: ctx.accounts.keeper.to_account_info(),
            },
        ),
        refund_amount,
    )?;

    // Re-credit the cycle and wind `next_cycle_at` back by one frequency step.
    // Underflow on either is protocol-level impossible (cycle was just drained,
    // so cycles_remaining < original and next_cycle_at was just bumped forward)
    // — we still use `checked_*` to fail loudly rather than wrap.
    //
    // Cap cycles_remaining at the owner's originally-signed schedule. Without
    // this, a buggy or compromised keeper could call `refund_cycle` repeatedly
    // (funding each call from its own ATA) to inflate cycles_remaining beyond
    // initial_num_cycles.
    require!(
        order.cycles_remaining < order.initial_num_cycles,
        DcaError::CycleOverRefund,
    );
    let order = &mut ctx.accounts.order;
    order.cycles_remaining = order
        .cycles_remaining
        .checked_add(1)
        .ok_or(DcaError::MathOverflow)?;
    order.next_cycle_at = order
        .next_cycle_at
        .checked_sub(order.cycle_frequency)
        .ok_or(DcaError::MathOverflow)?;

    let cycles_remaining = order.cycles_remaining;
    emit!(CycleRefunded {
        order: ctx.accounts.order.key(),
        cycles_remaining,
        refund_amount,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}
