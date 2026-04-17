mod helpers;

use {
    anchor_lang::prelude::Pubkey,
    helpers::*,
    solana_signer::Signer,
};

#[test]
fn test_initialize_config_success() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();

    env.initialize_config(100, fee_vault).unwrap();

    let config = env.read_config();
    assert_eq!(config.admin, env.admin.pubkey());
    assert_eq!(config.keeper, env.keeper.pubkey());
    assert_eq!(config.fee_vault, fee_vault);
    assert_eq!(config.fee_bps, 100);
    assert_eq!(config.min_fee_lamports, 0);
    assert_eq!(config.default_cycle_frequency, DEFAULT_FREQUENCY);
    assert_eq!(config.default_num_cycles, DEFAULT_NUM_CYCLES);
    assert!(!config.paused);
    assert!(config.pending_admin.is_none());
}

#[test]
fn test_initialize_config_fee_too_high() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();

    let res = env.initialize_config(10_001, fee_vault);
    assert!(res.is_err(), "should reject fee_bps > 10000");
}

#[test]
fn test_initialize_config_cannot_reinitialize() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();

    env.initialize_config(100, fee_vault).unwrap();
    let res = env.initialize_config(200, fee_vault);
    assert!(res.is_err(), "should not allow reinitializing config");
}

// M6: initialize must reject cycle_frequency above MAX_CYCLE_FREQUENCY (30 days).
#[test]
fn test_initialize_config_rejects_frequency_too_high() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();

    // 31 days > MAX_CYCLE_FREQUENCY.
    let res = env.initialize_config_full(100, 0, fee_vault, 31 * 24 * 60 * 60, DEFAULT_NUM_CYCLES);
    assert!(res.is_err(), "should reject cycle_frequency above MAX_CYCLE_FREQUENCY");
}

// M6: initialize must reject num_cycles above MAX_NUM_CYCLES (1_000).
#[test]
fn test_initialize_config_rejects_num_cycles_too_high() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();

    let res = env.initialize_config_full(100, 0, fee_vault, DEFAULT_FREQUENCY, 1_001);
    assert!(res.is_err(), "should reject num_cycles above MAX_NUM_CYCLES");
}

// Reject min_fee_lamports above MAX_MIN_FEE_LAMPORTS cap (1 SOL).
#[test]
fn test_initialize_config_rejects_min_fee_too_high() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();

    let res = env.initialize_config_full(
        45,
        2_000_000_000, // 2 SOL > 1 SOL cap
        fee_vault,
        DEFAULT_FREQUENCY,
        DEFAULT_NUM_CYCLES,
    );
    assert!(res.is_err(), "should reject min_fee_lamports above MAX_MIN_FEE_LAMPORTS");
}
