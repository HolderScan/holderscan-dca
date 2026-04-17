mod helpers;

use {
    anchor_lang::prelude::Pubkey,
    helpers::*,
    solana_keypair::Keypair,
    solana_signer::Signer,
};

// ── Update Config ──────────────────────────────────────────────────

#[test]
fn test_update_config_by_admin() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let new_keeper = Pubkey::new_unique();
    let new_vault = Pubkey::new_unique();
    let admin = env.admin.insecure_clone();
    env.update_config(&admin, Some(new_keeper), Some(new_vault), Some(50), Some(true)).unwrap();

    let config = env.read_config();
    assert_eq!(config.keeper, new_keeper);
    assert_eq!(config.fee_vault, new_vault);
    assert_eq!(config.fee_bps, 50);
    assert!(config.paused);
}

#[test]
fn test_update_config_partial() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let admin = env.admin.insecure_clone();
    env.update_config(&admin, None, None, None, Some(true)).unwrap();

    let config = env.read_config();
    assert_eq!(config.keeper, env.keeper.pubkey());
    assert_eq!(config.fee_vault, fee_vault);
    assert_eq!(config.fee_bps, 100);
    assert!(config.paused);
}

#[test]
fn test_update_config_rejects_non_admin() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let imposter = Keypair::new();
    env.svm.airdrop(&imposter.pubkey(), LAMPORTS_PER_SOL).unwrap();

    let res = env.update_config(&imposter, None, None, Some(0), None);
    assert!(res.is_err(), "non-admin should not update config");
}

#[test]
fn test_update_config_rejects_fee_too_high() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let admin = env.admin.insecure_clone();
    let res = env.update_config(&admin, None, None, Some(10_001), None);
    assert!(res.is_err(), "should reject fee_bps > 10000");
}

// Regression for H4: raising `default_num_cycles` alone past the current
// `min_total_in_amount` would leave `create_order` effectively broken (no valid
// `total_in_amount` could satisfy both the minimum and `per_cycle > 0`).
// The handler's final invariant check must reject this.
#[test]
fn test_update_config_rejects_num_cycles_above_min_total() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    // Test fixture sets min_total_in_amount == DEFAULT_NUM_CYCLES (42).
    // Raising num_cycles alone to 100 violates min_total >= num_cycles.
    let admin = env.admin.insecure_clone();
    let res = env.update_config_full(
        &admin, None, None, None, None, None, Some(100), None, None,
    );
    assert!(res.is_err(), "should reject num_cycles > min_total alone");
}

// M6: update must reject cycle_frequency above MAX_CYCLE_FREQUENCY (30 days).
#[test]
fn test_update_config_rejects_frequency_too_high() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let admin = env.admin.insecure_clone();
    let res = env.update_config_full(
        &admin, None, None, None, None, Some(31 * 24 * 60 * 60), None, None, None,
    );
    assert!(res.is_err(), "should reject cycle_frequency above MAX_CYCLE_FREQUENCY");
}

// M6: update must reject num_cycles above MAX_NUM_CYCLES (1_000). Pair it with
// a matching min_total so the H4 invariant doesn't mask the M6 failure.
#[test]
fn test_update_config_rejects_num_cycles_too_high() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let admin = env.admin.insecure_clone();
    let res = env.update_config_full(
        &admin, None, None, None, None, None, Some(1_001), Some(1_001), None,
    );
    assert!(res.is_err(), "should reject num_cycles above MAX_NUM_CYCLES");
}

// H4 counterpart: raising both `default_num_cycles` and `min_total_in_amount`
// in the same call must succeed as long as the final state preserves the
// invariant, regardless of the handler's internal field-processing order.
#[test]
fn test_update_config_accepts_num_cycles_and_min_total_together() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let admin = env.admin.insecure_clone();
    env.update_config_full(
        &admin, None, None, None, None, None, Some(100), Some(200), None,
    ).unwrap();

    let config = env.read_config();
    assert_eq!(config.default_num_cycles, 100);
    assert_eq!(config.min_total_in_amount, 200);
}

// Updating min_fee_lamports alone must validate the cap and land in state.
#[test]
fn test_update_config_rejects_min_fee_too_high() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(45, fee_vault).unwrap();

    let admin = env.admin.insecure_clone();
    let res = env.update_config_full(
        &admin, None, None, None, Some(2_000_000_000), None, None, None, None,
    );
    assert!(res.is_err(), "should reject min_fee_lamports > MAX_MIN_FEE_LAMPORTS");
}

#[test]
fn test_update_config_sets_min_fee_lamports() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(45, fee_vault).unwrap();

    let admin = env.admin.insecure_clone();
    env.update_config_full(
        &admin, None, None, None, Some(10_000_000), None, None, None, None,
    ).unwrap();

    let config = env.read_config();
    assert_eq!(config.fee_bps, 45);
    assert_eq!(config.min_fee_lamports, 10_000_000);
}

// ── Propose + Accept Admin ─────────────────────────────────────────

#[test]
fn test_two_step_admin_transfer() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let new_admin = Keypair::new();
    env.svm.airdrop(&new_admin.pubkey(), LAMPORTS_PER_SOL).unwrap();

    // Step 1: propose
    let admin = env.admin.insecure_clone();
    env.propose_admin(&admin, new_admin.pubkey()).unwrap();
    let config = env.read_config();
    assert_eq!(config.pending_admin, Some(new_admin.pubkey()));
    assert_eq!(config.admin, env.admin.pubkey()); // still old admin

    // Step 2: accept
    env.accept_admin(&new_admin).unwrap();
    let config = env.read_config();
    assert_eq!(config.admin, new_admin.pubkey());
    assert!(config.pending_admin.is_none());
}

#[test]
fn test_propose_admin_rejects_non_admin() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let imposter = Keypair::new();
    env.svm.airdrop(&imposter.pubkey(), LAMPORTS_PER_SOL).unwrap();

    let res = env.propose_admin(&imposter, imposter.pubkey());
    assert!(res.is_err(), "non-admin should not propose");
}

#[test]
fn test_accept_admin_rejects_wrong_signer() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let new_admin = Keypair::new();
    env.svm.airdrop(&new_admin.pubkey(), LAMPORTS_PER_SOL).unwrap();

    let admin = env.admin.insecure_clone();
    env.propose_admin(&admin, new_admin.pubkey()).unwrap();

    // Wrong person tries to accept
    let wrong = Keypair::new();
    env.svm.airdrop(&wrong.pubkey(), LAMPORTS_PER_SOL).unwrap();
    let res = env.accept_admin(&wrong);
    assert!(res.is_err(), "wrong signer should not accept");
}

#[test]
fn test_accept_admin_rejects_when_no_pending() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let someone = Keypair::new();
    env.svm.airdrop(&someone.pubkey(), LAMPORTS_PER_SOL).unwrap();

    let res = env.accept_admin(&someone);
    assert!(res.is_err(), "should fail when no pending admin");
}

#[test]
fn test_propose_admin_can_overwrite_pending() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let first = Keypair::new();
    let second = Keypair::new();

    let admin = env.admin.insecure_clone();
    env.propose_admin(&admin, first.pubkey()).unwrap();
    assert_eq!(env.read_config().pending_admin, Some(first.pubkey()));

    // Overwrite with a different proposal
    let admin = env.admin.insecure_clone();
    env.propose_admin(&admin, second.pubkey()).unwrap();
    assert_eq!(env.read_config().pending_admin, Some(second.pubkey()));
}

#[test]
fn test_old_admin_cannot_update_after_transfer() {
    let mut env = TestEnv::new();
    let fee_vault = Pubkey::new_unique();
    env.initialize_config(100, fee_vault).unwrap();

    let new_admin = Keypair::new();
    env.svm.airdrop(&new_admin.pubkey(), LAMPORTS_PER_SOL).unwrap();

    let admin = env.admin.insecure_clone();
    env.propose_admin(&admin, new_admin.pubkey()).unwrap();
    env.accept_admin(&new_admin).unwrap();

    // Old admin should now be rejected
    let old_admin = env.admin.insecure_clone();
    let res = env.update_config(&old_admin, None, None, None, Some(true));
    assert!(res.is_err(), "old admin should be rejected after transfer");

    // New admin should work
    env.update_config(&new_admin, None, None, None, Some(true)).unwrap();
    assert!(env.read_config().paused);
}
