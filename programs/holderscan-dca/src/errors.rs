use anchor_lang::prelude::*;

#[error_code]
pub enum DcaError {
    #[msg("Program is paused")]
    ProgramPaused,

    #[msg("Amount must be greater than zero")]
    InvalidAmount,

    #[msg("Total amount must be evenly divisible by number of cycles")]
    UnevenCycles,

    #[msg("Input mint must be Wrapped SOL")]
    InvalidInputMint,

    #[msg("Cycle frequency must be at least 60 seconds")]
    FrequencyTooLow,

    #[msg("Cycle frequency exceeds MAX_CYCLE_FREQUENCY cap")]
    FrequencyTooHigh,

    #[msg("Number of cycles exceeds MAX_NUM_CYCLES cap")]
    NumCyclesTooHigh,

    #[msg("created_at is outside the allowed tolerance window")]
    InvalidCreatedAt,

    #[msg("Cycle is not due yet")]
    CycleTooEarly,

    #[msg("Order is no longer active")]
    OrderInactive,

    #[msg("Order has no remaining cycles")]
    OrderComplete,

    #[msg("Unauthorized keeper")]
    UnauthorizedKeeper,

    #[msg("Fee basis points exceed MAX_FEE_BPS cap")]
    FeeTooHigh,

    #[msg("Fee tier thresholds must be strictly increasing")]
    InvalidFeeTiers,

    #[msg("Unauthorized admin")]
    UnauthorizedAdmin,

    #[msg("No pending admin transfer")]
    NoPendingAdmin,

    #[msg("Signer does not match pending admin")]
    PendingAdminMismatch,

    #[msg("Arithmetic overflow")]
    MathOverflow,

    #[msg("Total DCA amount is below the configured minimum")]
    TotalAmountBelowMinimum,

    #[msg("min_total_in_amount must be greater than or equal to default_num_cycles")]
    MinTotalBelowNumCycles,

    #[msg("refund would push cycles_remaining above initial_num_cycles")]
    CycleOverRefund,
}