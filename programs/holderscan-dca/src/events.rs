use anchor_lang::prelude::*;

#[event]
pub struct OrderCreated {
    pub order: Pubkey,
    pub owner: Pubkey,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub total_amount: u64,
    pub per_cycle: u64,
    pub cycles: u64,
    pub frequency: i64,
    pub fee_paid: u64,
}

#[event]
pub struct CycleExecuted {
    pub order: Pubkey,
    pub cycles_remaining: u64,
    pub swap_amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct OrderCancelled {
    pub order: Pubkey,
    pub owner: Pubkey,
    pub refunded_amount: u64,
}

#[event]
pub struct CycleRefunded {
    pub order: Pubkey,
    pub cycles_remaining: u64,
    pub refund_amount: u64,
    pub timestamp: i64,
}