use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Mint, Transfer, spl_token};
use anchor_spl::token_interface::Mint as MintInterface;
use crate::state::{DcaConfig, DcaOrder, CREATED_AT_TOLERANCE_SECS};
use crate::errors::DcaError;
use crate::events::OrderCreated;

#[derive(Accounts)]
#[instruction(total_in_amount: u64, created_at: i64)]
pub struct CreateOrder<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [b"dca_config"],
        bump = config.bump,
        constraint = !config.paused @ DcaError::ProgramPaused,
    )]
    pub config: Account<'info, DcaConfig>,

    #[account(
        constraint = input_mint.key() == spl_token::native_mint::ID @ DcaError::InvalidInputMint,
    )]
    pub input_mint: Account<'info, Mint>,
    /// Output mint accepted as either classic SPL Token or Token-2022.
    /// The program never CPIs against this mint — it's recorded on the order
    /// and used as a PDA seed. The keeper is responsible for honoring any
    /// Token-2022 extensions (transfer fees, hooks, etc.) during swap/payout.
    pub output_mint: InterfaceAccount<'info, MintInterface>,

    #[account(
        init,
        payer = owner,
        space = 8 + DcaOrder::INIT_SPACE,
        seeds = [
            b"dca_order",
            owner.key().as_ref(),
            input_mint.key().as_ref(),
            output_mint.key().as_ref(),
            &created_at.to_le_bytes(),
        ],
        bump,
    )]
    pub order: Account<'info, DcaOrder>,

    #[account(
        init,
        payer = owner,
        token::mint = input_mint,
        token::authority = escrow_authority,
        seeds = [b"escrow", order.key().as_ref()],
        bump,
    )]
    pub escrow_token_account: Account<'info, TokenAccount>,

    /// CHECK: PDA authority over the escrow — never signs directly
    #[account(
        seeds = [b"escrow_auth", order.key().as_ref()],
        bump,
    )]
    pub escrow_authority: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = user_input_ata.mint == input_mint.key(),
        constraint = user_input_ata.owner == owner.key(),
    )]
    pub user_input_ata: Account<'info, TokenAccount>,

    /// Fee vault receives the upfront, non-refundable platform fee
    #[account(
        mut,
        constraint = fee_vault.key() == config.fee_vault,
        constraint = fee_vault.mint == input_mint.key(),
    )]
    pub fee_vault: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<CreateOrder>,
    total_in_amount: u64,
    created_at: i64,
) -> Result<()> {
    let config = &ctx.accounts.config;

    // Schedule is a protocol-wide setting, not a per-order choice.
    let frequency = config.default_cycle_frequency;
    let cycles = config.default_num_cycles;

    let clock = Clock::get()?;

    // Bound the user-supplied `created_at` (used as an order-PDA seed) to a
    // tight window around on-chain time. Without this, callers could seed
    // arbitrary timestamps and inflate the per-(owner, mints) order set.
    require!(
        (created_at - clock.unix_timestamp).abs() <= CREATED_AT_TOLERANCE_SECS,
        DcaError::InvalidCreatedAt,
    );

    require!(
        total_in_amount >= config.min_total_in_amount,
        DcaError::TotalAmountBelowMinimum
    );
    require!(
        total_in_amount % cycles == 0,
        DcaError::UnevenCycles
    );

    let in_amount_per_cycle = total_in_amount / cycles;
    require!(in_amount_per_cycle > 0, DcaError::InvalidAmount);

    // Compute upfront, non-refundable fee: max(notional * fee_bps / 10_000, min_fee_lamports).
    // wSOL-only enforcement above means total_in_amount is always denominated in lamports.
    let fee_amount = config
        .compute_fee(total_in_amount)
        .ok_or(DcaError::MathOverflow)?;

    // Transfer DCA tokens from user to escrow
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.user_input_ata.to_account_info(),
                to: ctx.accounts.escrow_token_account.to_account_info(),
                authority: ctx.accounts.owner.to_account_info(),
            },
        ),
        total_in_amount,
    )?;

    // Transfer upfront fee from user to fee vault (non-refundable)
    if fee_amount > 0 {
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.user_input_ata.to_account_info(),
                    to: ctx.accounts.fee_vault.to_account_info(),
                    authority: ctx.accounts.owner.to_account_info(),
                },
            ),
            fee_amount,
        )?;
    }

    // Initialize order state
    let order = &mut ctx.accounts.order;
    order.owner = ctx.accounts.owner.key();
    order.input_mint = ctx.accounts.input_mint.key();
    order.output_mint = ctx.accounts.output_mint.key();
    order.in_amount_per_cycle = in_amount_per_cycle;
    order.cycles_remaining = cycles;
    order.initial_num_cycles = cycles;
    order.cycle_frequency = frequency;
    order.next_cycle_at = clock.unix_timestamp;
    order.is_active = true;
    order.bump = ctx.bumps.order;
    order.escrow_bump = ctx.bumps.escrow_token_account;

    emit!(OrderCreated {
        order: ctx.accounts.order.key(),
        owner: ctx.accounts.owner.key(),
        input_mint: ctx.accounts.input_mint.key(),
        output_mint: ctx.accounts.output_mint.key(),
        total_amount: total_in_amount,
        per_cycle: in_amount_per_cycle,
        cycles,
        frequency,
        fee_paid: fee_amount,
    });

    Ok(())
}
