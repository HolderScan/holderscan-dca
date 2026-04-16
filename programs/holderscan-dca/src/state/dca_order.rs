use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct DcaOrder {
    pub owner: Pubkey,           // who can cancel
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub in_amount_per_cycle: u64,
    pub cycles_remaining: u64,
    /// Snapshot of `cycles_remaining` at `create_order` time. Immutable after
    /// creation. Used by `refund_cycle` to cap `cycles_remaining` at the
    /// owner's originally-signed schedule so repeated refunds cannot inflate it.
    pub initial_num_cycles: u64,
    pub cycle_frequency: i64,    // seconds between cycles
    pub next_cycle_at: i64,      // unix timestamp
    pub is_active: bool,
    pub bump: u8,
    pub escrow_bump: u8,
}