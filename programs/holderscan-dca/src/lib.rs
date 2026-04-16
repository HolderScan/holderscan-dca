pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use instructions::*;
pub use state::FeeTiers;

declare_id!("2k7JFjY617MMCsshPMpRkYxR4Cx1gALPeFgNpfvCg4G5");

#[program]
pub mod holderscan_dca {
    use super::*;

    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        fee_vault: Pubkey,
        keeper: Pubkey,
        fee_tiers: FeeTiers,
        default_cycle_frequency: i64,
        default_num_cycles: u64,
        min_total_in_amount: u64,
    ) -> Result<()> {
        instructions::initialize_config::handler(
            ctx,
            fee_vault,
            keeper,
            fee_tiers,
            default_cycle_frequency,
            default_num_cycles,
            min_total_in_amount,
        )
    }

    pub fn update_config(
        ctx: Context<UpdateConfig>,
        new_keeper: Option<Pubkey>,
        new_fee_vault: Option<Pubkey>,
        new_fee_tiers: Option<FeeTiers>,
        new_default_cycle_frequency: Option<i64>,
        new_default_num_cycles: Option<u64>,
        new_min_total_in_amount: Option<u64>,
        paused: Option<bool>,
    ) -> Result<()> {
        instructions::update_config::handler(
            ctx,
            new_keeper,
            new_fee_vault,
            new_fee_tiers,
            new_default_cycle_frequency,
            new_default_num_cycles,
            new_min_total_in_amount,
            paused,
        )
    }

    pub fn propose_admin(ctx: Context<ProposeAdmin>, new_admin: Pubkey) -> Result<()> {
        instructions::propose_admin::handler(ctx, new_admin)
    }

    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        instructions::accept_admin::handler(ctx)
    }

    pub fn create_order(
        ctx: Context<CreateOrder>,
        total_in_amount: u64,
        created_at: i64,
    ) -> Result<()> {
        instructions::create_order::handler(ctx, total_in_amount, created_at)
    }

    pub fn execute_cycle(ctx: Context<ExecuteCycle>) -> Result<()> {
        instructions::execute_cycle::handler(ctx)
    }

    pub fn refund_cycle(ctx: Context<RefundCycle>) -> Result<()> {
        instructions::refund_cycle::handler(ctx)
    }

    pub fn cancel_order(ctx: Context<CancelOrder>) -> Result<()> {
        instructions::cancel_order::handler(ctx)
    }
}
