mod helpers;

use {
    anchor_lang::prelude::Pubkey,
    helpers::*,
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const FEE_BPS: u16 = 100; // 1%
const CREATED_AT: i64 = 1_000_000;

// Use small cycle counts for tests (not the real 42)
const TEST_CYCLES: u64 = 10;
const TEST_FREQUENCY: i64 = 3600; // 1 hour
const TOTAL_AMOUNT: u64 = 1_000_000; // per_cycle = 100_000
const PER_CYCLE: u64 = TOTAL_AMOUNT / TEST_CYCLES;
const FEE_AMOUNT: u64 = TOTAL_AMOUNT * FEE_BPS as u64 / 10_000; // 10_000

/// Sets up a test environment with WSOL mint, output mint, config, and funded user.
fn setup_order_env() -> (TestEnv, Keypair, Pubkey, Pubkey, Pubkey, Pubkey) {
    let mut env = TestEnv::new();

    let output_mint = Pubkey::new_unique();
    let fee_vault = Pubkey::new_unique();

    // Use test frequency/cycles instead of the 4h/42 defaults. min_fee=0 keeps
    // the existing bps-based FEE_AMOUNT assertions intact.
    env.initialize_config_full(FEE_BPS, 0, fee_vault, TEST_FREQUENCY, TEST_CYCLES).unwrap();

    // Create WSOL mint at the real address + output mint
    env.create_wsol_mint();
    let mint_authority = Pubkey::new_unique();
    env.create_mint(&output_mint, &mint_authority);

    // Create user + fund their WSOL ATA (DCA notional + upfront fee)
    let user = Keypair::new();
    env.svm.airdrop(&user.pubkey(), 100 * LAMPORTS_PER_SOL).unwrap();
    let user_ata = Pubkey::new_unique();
    env.create_token_account(&user_ata, &wsol_mint(), &user.pubkey(), TOTAL_AMOUNT + FEE_AMOUNT);

    // Create fee vault token account (WSOL)
    let fee_vault_owner = Pubkey::new_unique();
    env.create_token_account(&fee_vault, &wsol_mint(), &fee_vault_owner, 0);

    // Create keeper's WSOL ATA
    let keeper_ata = Pubkey::new_unique();
    env.create_token_account(&keeper_ata, &wsol_mint(), &env.keeper.pubkey(), 0);

    (env, user, output_mint, user_ata, fee_vault, keeper_ata)
}

// ── Create Order ───────────────────────────────────────────────────

#[test]
fn test_create_order_success() {
    let (mut env, user, output_mint, user_ata, fee_vault, _keeper_ata) = setup_order_env();

    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    let order_pda = env.wsol_order_pda(&user.pubkey(), &output_mint, CREATED_AT);
    let order = env.read_order(&order_pda);
    assert_eq!(order.owner, user.pubkey());
    assert_eq!(order.input_mint, wsol_mint());
    assert_eq!(order.output_mint, output_mint);
    assert_eq!(order.in_amount_per_cycle, PER_CYCLE);
    assert_eq!(order.cycles_remaining, TEST_CYCLES);
    assert_eq!(order.cycle_frequency, TEST_FREQUENCY);
    assert!(order.is_active);

    // User's ATA should be empty (DCA to escrow + fee to vault)
    assert_eq!(env.read_token_balance(&user_ata), 0);
    assert_eq!(env.read_token_balance(&fee_vault), FEE_AMOUNT);
    let (escrow_pda, _) = Pubkey::find_program_address(
        &[b"escrow", order_pda.as_ref()],
        &env.program_id,
    );
    assert_eq!(env.read_token_balance(&escrow_pda), TOTAL_AMOUNT);
}

// H1-lite: refund_cycle must not push cycles_remaining above initial_num_cycles.
// At order creation the two are equal, so any refund call before an execute_cycle
// would violate the cap and must be rejected. Fund the keeper's WSOL ATA with
// exactly one cycle so the SPL transfer itself would otherwise succeed — this
// isolates the cap check from incidental insufficient-balance failures.
#[test]
fn test_refund_cycle_rejected_when_at_initial_num_cycles() {
    let (mut env, user, output_mint, user_ata, fee_vault, keeper_ata) = setup_order_env();

    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    // Give keeper ATA a cycle's worth of wSOL so the transfer would succeed
    // on its own; the failure must come from the CycleOverRefund cap, not from
    // an incidental SPL transfer error.
    env.create_token_account(&keeper_ata, &wsol_mint(), &env.keeper.pubkey(), PER_CYCLE);

    let order_pda = env.wsol_order_pda(&user.pubkey(), &output_mint, CREATED_AT);
    let res = env.refund_cycle(order_pda, keeper_ata, user.pubkey(), user_ata);
    assert!(res.is_err(), "refund at initial_num_cycles should be rejected");
}

// M1: `created_at` seeds the order PDA; it must track the on-chain clock within
// CREATED_AT_TOLERANCE_SECS so callers can't spam arbitrary-timestamp order PDAs.
#[test]
fn test_create_order_rejects_created_at_out_of_window() {
    let (mut env, user, output_mint, user_ata, fee_vault, _keeper_ata) = setup_order_env();

    env.set_clock(CREATED_AT);

    // CREATED_AT_TOLERANCE_SECS = 60; +61s is outside the window.
    let bad_created_at = CREATED_AT + 61;
    let res = env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, bad_created_at);
    assert!(res.is_err(), "should reject created_at outside tolerance window");
}

#[test]
fn test_create_order_rejects_non_wsol_input() {
    let (mut env, user, output_mint, _user_ata, fee_vault, _keeper_ata) = setup_order_env();

    // Create a non-WSOL mint and ATA
    let fake_mint = Pubkey::new_unique();
    let mint_authority = Pubkey::new_unique();
    env.create_mint(&fake_mint, &mint_authority);
    let fake_ata = Pubkey::new_unique();
    env.create_token_account(&fake_ata, &fake_mint, &user.pubkey(), TOTAL_AMOUNT);

    env.set_clock(CREATED_AT);

    // Build tx manually with fake_mint as input_mint
    let order_pda = env.order_pda(&user.pubkey(), &fake_mint, &output_mint, CREATED_AT);
    let (escrow_pda, _) = Pubkey::find_program_address(
        &[b"escrow", order_pda.as_ref()],
        &env.program_id,
    );
    let (escrow_auth, _) = Pubkey::find_program_address(
        &[b"escrow_auth", order_pda.as_ref()],
        &env.program_id,
    );

    use anchor_lang::{InstructionData, ToAccountMetas, solana_program::instruction::Instruction};
    use spl_token_interface::ID as TOKEN_PROGRAM_ID;

    let ix = Instruction::new_with_bytes(
        env.program_id,
        &holderscan_dca::instruction::CreateOrder {
            total_in_amount: TOTAL_AMOUNT,
            created_at: CREATED_AT,
        }.data(),
        holderscan_dca::accounts::CreateOrder {
            owner: user.pubkey(),
            config: env.config_pda(),
            input_mint: fake_mint,
            output_mint,
            order: order_pda,
            escrow_token_account: escrow_pda,
            escrow_authority: escrow_auth,
            user_input_ata: fake_ata,
            fee_vault,
            token_program: TOKEN_PROGRAM_ID,
            system_program: anchor_lang::solana_program::system_program::id(),
        }.to_account_metas(None),
    );

    let blockhash = env.svm.latest_blockhash();
    use solana_message::{Message, VersionedMessage};
    use solana_transaction::versioned::VersionedTransaction;
    let msg = Message::new_with_blockhash(&[ix], Some(&user.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[&user]).unwrap();
    let res = env.svm.send_transaction(tx);
    assert!(res.is_err(), "should reject non-WSOL input mint");
}

#[test]
fn test_create_order_rejects_zero_amount() {
    let (mut env, user, output_mint, user_ata, fee_vault, _keeper_ata) = setup_order_env();

    env.set_clock(CREATED_AT);
    let res = env.create_order(&user, output_mint, user_ata, fee_vault, 0, CREATED_AT);
    assert!(res.is_err(), "should reject zero total amount");
}

#[test]
fn test_create_order_rejects_uneven_cycles() {
    let (mut env, user, output_mint, user_ata, fee_vault, _keeper_ata) = setup_order_env();

    // 1_000_001 / 10 cycles doesn't divide evenly
    env.create_token_account(&user_ata, &wsol_mint(), &user.pubkey(), 1_000_001);
    env.set_clock(CREATED_AT);
    let res = env.create_order(&user, output_mint, user_ata, fee_vault, 1_000_001, CREATED_AT);
    assert!(res.is_err(), "should reject uneven cycles");
}

// Fee floor: when min_fee_lamports > notional * fee_bps / 10_000, the floor
// wins. Raise min_fee above the bps fee (10_000) and verify the vault gets
// the floor amount instead.
#[test]
fn test_create_order_charges_min_fee_floor() {
    let (mut env, user, output_mint, user_ata, fee_vault, _keeper_ata) = setup_order_env();

    // Bump the floor to 50_000 (> pct fee of 10_000 on TOTAL_AMOUNT of 1_000_000).
    let admin = env.admin.insecure_clone();
    let floor: u64 = 50_000;
    env.update_config_full(
        &admin, None, None, None, Some(floor), None, None, None, None,
    ).unwrap();

    // Top up user's ATA so they can cover notional + floor fee.
    env.create_token_account(&user_ata, &wsol_mint(), &user.pubkey(), TOTAL_AMOUNT + floor);

    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    // Floor paid, not bps fee.
    assert_eq!(env.read_token_balance(&fee_vault), floor);
}

#[test]
fn test_create_order_rejects_when_paused() {
    let (mut env, user, output_mint, user_ata, fee_vault, _keeper_ata) = setup_order_env();

    let admin = env.admin.insecure_clone();
    env.update_config(&admin, None, None, None, Some(true)).unwrap();

    env.set_clock(CREATED_AT);
    let res = env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT);
    assert!(res.is_err(), "should reject when program is paused");
}

// ── Execute Cycle ──────────────────────────────────────────────────

#[test]
fn test_execute_cycle_success() {
    let (mut env, user, output_mint, user_ata, fee_vault, keeper_ata) = setup_order_env();

    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    // Fee collected upfront: 1_000_000 * 100 / 10_000 = 10_000
    assert_eq!(env.read_token_balance(&fee_vault), FEE_AMOUNT);

    let order_pda = env.wsol_order_pda(&user.pubkey(), &output_mint, CREATED_AT);

    env.execute_cycle(order_pda, keeper_ata, user.pubkey(), user_ata).unwrap();

    let order = env.read_order(&order_pda);
    assert_eq!(order.cycles_remaining, TEST_CYCLES - 1);
    assert_eq!(order.next_cycle_at, CREATED_AT + TEST_FREQUENCY);
    assert!(order.is_active);

    // Full cycle amount goes to keeper; fee already collected at creation
    assert_eq!(env.read_token_balance(&keeper_ata), PER_CYCLE);
    assert_eq!(env.read_token_balance(&fee_vault), FEE_AMOUNT);
}

#[test]
fn test_execute_cycle_too_early() {
    let (mut env, user, output_mint, user_ata, fee_vault, keeper_ata) = setup_order_env();

    // Create order with custom overrides to start in the future
    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    let order_pda = env.wsol_order_pda(&user.pubkey(), &output_mint, CREATED_AT);

    // Execute first cycle (this one works — next_cycle_at = CREATED_AT)
    env.execute_cycle(order_pda, keeper_ata, user.pubkey(), user_ata).unwrap();

    // Try again immediately — should fail, next_cycle_at is now CREATED_AT + FREQUENCY
    env.svm.expire_blockhash();
    let res = env.execute_cycle(order_pda, keeper_ata, user.pubkey(), user_ata);
    assert!(res.is_err(), "should reject cycle before next_cycle_at");
}

#[test]
fn test_execute_final_cycle_closes_order() {
    let (mut env, user, output_mint, user_ata, fee_vault, keeper_ata) = setup_order_env();

    // Schedule is global-only now; drain all TEST_CYCLES cycles to hit the final one.
    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    let order_pda = env.wsol_order_pda(&user.pubkey(), &output_mint, CREATED_AT);
    let (escrow_pda, _) = Pubkey::find_program_address(
        &[b"escrow", order_pda.as_ref()],
        &env.program_id,
    );

    // Cycles 1..=TEST_CYCLES-1 leave the order active.
    for i in 0..TEST_CYCLES - 1 {
        env.set_clock(CREATED_AT + (i as i64) * TEST_FREQUENCY);
        env.svm.expire_blockhash();
        env.execute_cycle(order_pda, keeper_ata, user.pubkey(), user_ata).unwrap();
        assert!(env.read_order(&order_pda).is_active);
    }

    // Final cycle — order and escrow are closed, rent returned to owner.
    env.set_clock(CREATED_AT + ((TEST_CYCLES - 1) as i64) * TEST_FREQUENCY);
    env.svm.expire_blockhash();
    env.execute_cycle(order_pda, keeper_ata, user.pubkey(), user_ata).unwrap();

    assert!(env.svm.get_account(&order_pda).is_none(), "order account should be closed");
    assert!(env.svm.get_account(&escrow_pda).is_none(), "escrow token account should be closed");
}

#[test]
fn test_execute_cycle_rejects_when_paused() {
    let (mut env, user, output_mint, user_ata, fee_vault, keeper_ata) = setup_order_env();

    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    let order_pda = env.wsol_order_pda(&user.pubkey(), &output_mint, CREATED_AT);

    let admin = env.admin.insecure_clone();
    env.update_config(&admin, None, None, None, Some(true)).unwrap();

    let res = env.execute_cycle(order_pda, keeper_ata, user.pubkey(), user_ata);
    assert!(res.is_err(), "should reject execution when paused");
}

#[test]
fn test_execute_cycle_rejects_non_keeper() {
    let (mut env, user, output_mint, user_ata, fee_vault, _keeper_ata) = setup_order_env();

    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    let order_pda = env.wsol_order_pda(&user.pubkey(), &output_mint, CREATED_AT);

    let fake_keeper = Keypair::new();
    env.svm.airdrop(&fake_keeper.pubkey(), LAMPORTS_PER_SOL).unwrap();
    let fake_keeper_ata = Pubkey::new_unique();
    env.create_token_account(&fake_keeper_ata, &wsol_mint(), &fake_keeper.pubkey(), 0);

    env.keeper = fake_keeper;
    let res = env.execute_cycle(order_pda, fake_keeper_ata, user.pubkey(), user_ata);
    assert!(res.is_err(), "non-keeper should be rejected");
}

// ── Cancel Order ───────────────────────────────────────────────────

#[test]
fn test_cancel_order_full_refund() {
    let (mut env, user, output_mint, user_ata, fee_vault, _keeper_ata) = setup_order_env();

    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    let order_pda = env.wsol_order_pda(&user.pubkey(), &output_mint, CREATED_AT);

    env.cancel_order(&user, order_pda, user_ata).unwrap();

    assert_eq!(env.read_token_balance(&user_ata), TOTAL_AMOUNT);
    assert!(env.svm.get_account(&order_pda).is_none());
}

#[test]
fn test_cancel_order_partial_refund_after_cycles() {
    let (mut env, user, output_mint, user_ata, fee_vault, keeper_ata) = setup_order_env();

    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    let order_pda = env.wsol_order_pda(&user.pubkey(), &output_mint, CREATED_AT);

    // Execute 3 cycles
    env.execute_cycle(order_pda, keeper_ata, user.pubkey(), user_ata).unwrap();
    env.set_clock(CREATED_AT + TEST_FREQUENCY);
    env.svm.expire_blockhash();
    env.execute_cycle(order_pda, keeper_ata, user.pubkey(), user_ata).unwrap();
    env.set_clock(CREATED_AT + 2 * TEST_FREQUENCY);
    env.svm.expire_blockhash();
    env.execute_cycle(order_pda, keeper_ata, user.pubkey(), user_ata).unwrap();

    // Cancel — should get remaining 7 cycles worth
    env.cancel_order(&user, order_pda, user_ata).unwrap();

    let expected_remaining = TOTAL_AMOUNT - (3 * PER_CYCLE);
    assert_eq!(env.read_token_balance(&user_ata), expected_remaining);
}

#[test]
fn test_cancel_order_rejects_non_owner() {
    let (mut env, user, output_mint, user_ata, fee_vault, _keeper_ata) = setup_order_env();

    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    let order_pda = env.wsol_order_pda(&user.pubkey(), &output_mint, CREATED_AT);

    let attacker = Keypair::new();
    env.svm.airdrop(&attacker.pubkey(), LAMPORTS_PER_SOL).unwrap();
    let attacker_ata = Pubkey::new_unique();
    env.create_token_account(&attacker_ata, &wsol_mint(), &attacker.pubkey(), 0);

    let res = env.cancel_order(&attacker, order_pda, attacker_ata);
    assert!(res.is_err(), "non-owner should not cancel order");
}

#[test]
fn test_cancel_order_cannot_cancel_twice() {
    let (mut env, user, output_mint, user_ata, fee_vault, _keeper_ata) = setup_order_env();

    env.set_clock(CREATED_AT);
    env.create_order(&user, output_mint, user_ata, fee_vault, TOTAL_AMOUNT, CREATED_AT).unwrap();

    let order_pda = env.wsol_order_pda(&user.pubkey(), &output_mint, CREATED_AT);

    env.cancel_order(&user, order_pda, user_ata).unwrap();

    let res = env.cancel_order(&user, order_pda, user_ata);
    assert!(res.is_err(), "should not cancel a closed order");
}
