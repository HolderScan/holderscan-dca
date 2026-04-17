use {
    anchor_lang::{
        prelude::Pubkey,
        solana_program::{
            instruction::Instruction,
            system_program,
        },
        AnchorDeserialize, InstructionData, ToAccountMetas,
    },
    litesvm::LiteSVM,
    solana_account::Account,
    solana_clock::Clock,
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_program_option::COption,
    solana_program_pack::Pack,
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
    spl_token_interface::{
        state::{Account as SplTokenAccount, AccountState, Mint as SplMint},
        ID as TOKEN_PROGRAM_ID,
    },
};

pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

/// Default min_fee_lamports for tests. Most fixtures use small `TOTAL_AMOUNT`s
/// (e.g. 1_000_000 lamports) where a non-zero floor would dominate the bps fee
/// and break existing assertions; keep it at 0 so the percentage fee is what
/// gets exercised. Tests that want to exercise the floor call the `_full`
/// helpers and pass their own value.
pub const DEFAULT_MIN_FEE_LAMPORTS: u64 = 0;

/// Wrapped SOL mint address
pub fn wsol_mint() -> Pubkey {
    spl_token_interface::native_mint::id()
}

/// Default cycle frequency: 4 hours
pub const DEFAULT_FREQUENCY: i64 = 14_400;
/// Default number of cycles: 42 (7 days at 4h intervals)
pub const DEFAULT_NUM_CYCLES: u64 = 42;

pub struct TestEnv {
    pub svm: LiteSVM,
    pub admin: Keypair,
    pub keeper: Keypair,
    pub program_id: Pubkey,
}

impl TestEnv {
    pub fn new() -> Self {
        let program_id = holderscan_dca::id();
        let admin = Keypair::new();
        let keeper = Keypair::new();
        let mut svm = LiteSVM::new().with_sysvars();
        let bytes = include_bytes!("../../../../target/deploy/holderscan_dca.so");
        svm.add_program(program_id, bytes).unwrap();
        svm.airdrop(&admin.pubkey(), 100 * LAMPORTS_PER_SOL).unwrap();
        svm.airdrop(&keeper.pubkey(), 100 * LAMPORTS_PER_SOL).unwrap();

        Self { svm, admin, keeper, program_id }
    }

    pub fn config_pda(&self) -> Pubkey {
        Pubkey::find_program_address(&[b"dca_config"], &self.program_id).0
    }

    // ── Initialize Config ──────────────────────────────────────────

    pub fn initialize_config(
        &mut self,
        fee_bps: u16,
        fee_vault: Pubkey,
    ) -> Result<(), String> {
        self.initialize_config_full(
            fee_bps,
            DEFAULT_MIN_FEE_LAMPORTS,
            fee_vault,
            DEFAULT_FREQUENCY,
            DEFAULT_NUM_CYCLES,
        )
    }

    pub fn initialize_config_full(
        &mut self,
        fee_bps: u16,
        min_fee_lamports: u64,
        fee_vault: Pubkey,
        default_cycle_frequency: i64,
        default_num_cycles: u64,
    ) -> Result<(), String> {
        let config_pda = self.config_pda();
        let admin_pubkey = self.admin.pubkey();
        let keeper_pubkey = self.keeper.pubkey();
        let admin = self.admin.insecure_clone();
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &holderscan_dca::instruction::InitializeConfig {
                fee_vault,
                keeper: keeper_pubkey,
                fee_bps,
                min_fee_lamports,
                default_cycle_frequency,
                default_num_cycles,
                // test default: tiny floor so existing tiny-amount fixtures still pass.
                min_total_in_amount: default_num_cycles,
            }.data(),
            holderscan_dca::accounts::InitializeConfig {
                admin: admin_pubkey,
                config: config_pda,
                system_program: system_program::id(),
            }.to_account_metas(None),
        );
        send_tx(&mut self.svm, &[ix], &[&admin])
    }

    // ── Update Config ──────────────────────────────────────────────

    pub fn update_config(
        &mut self,
        signer: &Keypair,
        new_keeper: Option<Pubkey>,
        new_fee_vault: Option<Pubkey>,
        new_fee_bps: Option<u16>,
        paused: Option<bool>,
    ) -> Result<(), String> {
        self.update_config_full(
            signer, new_keeper, new_fee_vault, new_fee_bps, None, None, None, None, paused,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_config_full(
        &mut self,
        signer: &Keypair,
        new_keeper: Option<Pubkey>,
        new_fee_vault: Option<Pubkey>,
        new_fee_bps: Option<u16>,
        new_min_fee_lamports: Option<u64>,
        new_default_cycle_frequency: Option<i64>,
        new_default_num_cycles: Option<u64>,
        new_min_total_in_amount: Option<u64>,
        paused: Option<bool>,
    ) -> Result<(), String> {
        let config_pda = self.config_pda();
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &holderscan_dca::instruction::UpdateConfig {
                new_keeper,
                new_fee_vault,
                new_fee_bps,
                new_min_fee_lamports,
                new_default_cycle_frequency,
                new_default_num_cycles,
                new_min_total_in_amount,
                paused,
            }.data(),
            holderscan_dca::accounts::UpdateConfig {
                admin: signer.pubkey(),
                config: config_pda,
            }.to_account_metas(None),
        );
        send_tx(&mut self.svm, &[ix], &[signer])
    }

    // ── Propose Admin ──────────────────────────────────────────────

    pub fn propose_admin(
        &mut self,
        signer: &Keypair,
        new_admin: Pubkey,
    ) -> Result<(), String> {
        let config_pda = self.config_pda();
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &holderscan_dca::instruction::ProposeAdmin {
                new_admin,
            }.data(),
            holderscan_dca::accounts::ProposeAdmin {
                admin: signer.pubkey(),
                config: config_pda,
            }.to_account_metas(None),
        );
        send_tx(&mut self.svm, &[ix], &[signer])
    }

    // ── Accept Admin ───────────────────────────────────────────────

    pub fn accept_admin(
        &mut self,
        signer: &Keypair,
    ) -> Result<(), String> {
        let config_pda = self.config_pda();
        let ix = Instruction::new_with_bytes(
            self.program_id,
            &holderscan_dca::instruction::AcceptAdmin {}.data(),
            holderscan_dca::accounts::AcceptAdmin {
                new_admin: signer.pubkey(),
                config: config_pda,
            }.to_account_metas(None),
        );
        send_tx(&mut self.svm, &[ix], &[signer])
    }

    // ── Create Order ───────────────────────────────────────────────

    /// Create an order. Schedule is protocol-wide (set via config), so there
    /// are no per-order overrides — tests that want a different schedule must
    /// reinitialize the config in their fixture.
    #[allow(clippy::too_many_arguments)]
    pub fn create_order(
        &mut self,
        owner: &Keypair,
        output_mint: Pubkey,
        user_input_ata: Pubkey,
        fee_vault: Pubkey,
        total_in_amount: u64,
        created_at: i64,
    ) -> Result<(), String> {
        let input_mint = wsol_mint();
        let order_pda = self.order_pda(&owner.pubkey(), &input_mint, &output_mint, created_at);
        let (escrow_pda, _) = Pubkey::find_program_address(
            &[b"escrow", order_pda.as_ref()],
            &self.program_id,
        );
        let (escrow_auth, _) = Pubkey::find_program_address(
            &[b"escrow_auth", order_pda.as_ref()],
            &self.program_id,
        );
        let config_pda = self.config_pda();

        let ix = Instruction::new_with_bytes(
            self.program_id,
            &holderscan_dca::instruction::CreateOrder {
                total_in_amount,
                created_at,
            }.data(),
            holderscan_dca::accounts::CreateOrder {
                owner: owner.pubkey(),
                config: config_pda,
                input_mint,
                output_mint,
                order: order_pda,
                escrow_token_account: escrow_pda,
                escrow_authority: escrow_auth,
                user_input_ata,
                fee_vault,
                token_program: TOKEN_PROGRAM_ID,
                system_program: system_program::id(),
            }.to_account_metas(None),
        );
        send_tx(&mut self.svm, &[ix], &[owner])
    }

    // ── Execute Cycle ──────────────────────────────────────────────

    pub fn execute_cycle(
        &mut self,
        order_pda: Pubkey,
        keeper_input_ata: Pubkey,
        owner: Pubkey,
        user_input_ata: Pubkey,
    ) -> Result<(), String> {
        let (escrow_pda, _) = Pubkey::find_program_address(
            &[b"escrow", order_pda.as_ref()],
            &self.program_id,
        );
        let (escrow_auth, _) = Pubkey::find_program_address(
            &[b"escrow_auth", order_pda.as_ref()],
            &self.program_id,
        );
        let config_pda = self.config_pda();
        let keeper_pubkey = self.keeper.pubkey();
        let keeper = self.keeper.insecure_clone();

        let ix = Instruction::new_with_bytes(
            self.program_id,
            &holderscan_dca::instruction::ExecuteCycle {}.data(),
            holderscan_dca::accounts::ExecuteCycle {
                keeper: keeper_pubkey,
                config: config_pda,
                order: order_pda,
                owner,
                escrow_token_account: escrow_pda,
                escrow_authority: escrow_auth,
                user_input_ata,
                keeper_input_ata,
                token_program: TOKEN_PROGRAM_ID,
            }.to_account_metas(None),
        );
        send_tx(&mut self.svm, &[ix], &[&keeper])
    }

    // ── Refund Cycle ───────────────────────────────────────────────

    pub fn refund_cycle(
        &mut self,
        order_pda: Pubkey,
        keeper_input_ata: Pubkey,
        owner: Pubkey,
        user_input_ata: Pubkey,
    ) -> Result<(), String> {
        let config_pda = self.config_pda();
        let keeper_pubkey = self.keeper.pubkey();
        let keeper = self.keeper.insecure_clone();

        let ix = Instruction::new_with_bytes(
            self.program_id,
            &holderscan_dca::instruction::RefundCycle {}.data(),
            holderscan_dca::accounts::RefundCycle {
                keeper: keeper_pubkey,
                config: config_pda,
                order: order_pda,
                owner,
                keeper_input_ata,
                user_input_ata,
                token_program: TOKEN_PROGRAM_ID,
            }.to_account_metas(None),
        );
        send_tx(&mut self.svm, &[ix], &[&keeper])
    }

    // ── Cancel Order ───────────────────────────────────────────────

    pub fn cancel_order(
        &mut self,
        owner: &Keypair,
        order_pda: Pubkey,
        user_input_ata: Pubkey,
    ) -> Result<(), String> {
        let (escrow_pda, _) = Pubkey::find_program_address(
            &[b"escrow", order_pda.as_ref()],
            &self.program_id,
        );
        let (escrow_auth, _) = Pubkey::find_program_address(
            &[b"escrow_auth", order_pda.as_ref()],
            &self.program_id,
        );

        let ix = Instruction::new_with_bytes(
            self.program_id,
            &holderscan_dca::instruction::CancelOrder {}.data(),
            holderscan_dca::accounts::CancelOrder {
                owner: owner.pubkey(),
                order: order_pda,
                escrow_token_account: escrow_pda,
                escrow_authority: escrow_auth,
                user_input_ata,
                token_program: TOKEN_PROGRAM_ID,
            }.to_account_metas(None),
        );
        send_tx(&mut self.svm, &[ix], &[owner])
    }

    // ── Read helpers ───────────────────────────────────────────────

    pub fn read_config(&self) -> holderscan_dca::state::DcaConfig {
        let acc = self.svm.get_account(&self.config_pda()).unwrap();
        let mut data = &acc.data[8..];
        holderscan_dca::state::DcaConfig::deserialize(&mut data).unwrap()
    }

    pub fn read_order(&self, order_pda: &Pubkey) -> holderscan_dca::state::DcaOrder {
        let acc = self.svm.get_account(order_pda).unwrap();
        let mut data = &acc.data[8..];
        holderscan_dca::state::DcaOrder::deserialize(&mut data).unwrap()
    }

    pub fn read_token_balance(&self, address: &Pubkey) -> u64 {
        let acc = self.svm.get_account(address).unwrap();
        SplTokenAccount::unpack(&acc.data).unwrap().amount
    }

    // ── Token helpers ──────────────────────────────────────────────

    pub fn create_mint(&mut self, mint_address: &Pubkey, authority: &Pubkey) {
        let mint = SplMint {
            mint_authority: COption::Some(*authority),
            supply: 0,
            decimals: 6,
            is_initialized: true,
            freeze_authority: COption::None,
        };
        let mut data = [0u8; SplMint::LEN];
        SplMint::pack(mint, &mut data).unwrap();
        let rent = self.svm.minimum_balance_for_rent_exemption(SplMint::LEN);
        self.svm.set_account(
            *mint_address,
            Account {
                lamports: rent,
                data: data.to_vec(),
                owner: TOKEN_PROGRAM_ID,
                executable: false,
                rent_epoch: u64::MAX,
            },
        ).unwrap();
    }

    /// Create WSOL mint at the real WSOL address (9 decimals)
    pub fn create_wsol_mint(&mut self) {
        let mint = SplMint {
            mint_authority: COption::None,
            supply: 0,
            decimals: 9,
            is_initialized: true,
            freeze_authority: COption::None,
        };
        let mut data = [0u8; SplMint::LEN];
        SplMint::pack(mint, &mut data).unwrap();
        let rent = self.svm.minimum_balance_for_rent_exemption(SplMint::LEN);
        self.svm.set_account(
            wsol_mint(),
            Account {
                lamports: rent,
                data: data.to_vec(),
                owner: TOKEN_PROGRAM_ID,
                executable: false,
                rent_epoch: u64::MAX,
            },
        ).unwrap();
    }

    pub fn create_token_account(
        &mut self,
        address: &Pubkey,
        mint: &Pubkey,
        owner: &Pubkey,
        amount: u64,
    ) {
        let rent = self.svm.minimum_balance_for_rent_exemption(SplTokenAccount::LEN);
        // A WSOL token account is "native": spl-token tracks lamports == rent_reserve + amount
        // and moves real SOL on transfers. Non-WSOL mints use the non-native layout.
        let is_native_mint = *mint == wsol_mint();
        let (is_native_field, lamports) = if is_native_mint {
            (COption::Some(rent), rent + amount)
        } else {
            (COption::None, rent)
        };
        let token_acc = SplTokenAccount {
            mint: *mint,
            owner: *owner,
            amount,
            delegate: COption::None,
            state: AccountState::Initialized,
            is_native: is_native_field,
            delegated_amount: 0,
            close_authority: COption::None,
        };
        let mut data = [0u8; SplTokenAccount::LEN];
        SplTokenAccount::pack(token_acc, &mut data).unwrap();
        self.svm.set_account(
            *address,
            Account {
                lamports,
                data: data.to_vec(),
                owner: TOKEN_PROGRAM_ID,
                executable: false,
                rent_epoch: u64::MAX,
            },
        ).unwrap();
    }

    // ── Clock helpers ──────────────────────────────────────────────

    pub fn set_clock(&mut self, unix_timestamp: i64) {
        let clock = Clock {
            slot: 1,
            epoch_start_timestamp: unix_timestamp,
            epoch: 0,
            leader_schedule_epoch: 0,
            unix_timestamp,
        };
        self.svm.set_sysvar(&clock);
    }

    // ── PDA helpers ────────────────────────────────────────────────

    pub fn order_pda(
        &self,
        owner: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
        created_at: i64,
    ) -> Pubkey {
        Pubkey::find_program_address(
            &[
                b"dca_order",
                owner.as_ref(),
                input_mint.as_ref(),
                output_mint.as_ref(),
                &created_at.to_le_bytes(),
            ],
            &self.program_id,
        ).0
    }

    /// Convenience: order PDA with WSOL as input mint
    pub fn wsol_order_pda(
        &self,
        owner: &Pubkey,
        output_mint: &Pubkey,
        created_at: i64,
    ) -> Pubkey {
        self.order_pda(owner, &wsol_mint(), output_mint, created_at)
    }
}

// ── Free-standing tx sender ────────────────────────────────────────

fn send_tx(
    svm: &mut LiteSVM,
    ixs: &[Instruction],
    signers: &[&Keypair],
) -> Result<(), String> {
    let payer = signers[0].pubkey();
    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(ixs, Some(&payer), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx)
        .map(|_| ())
        .map_err(|e| format!("{:?}", e))
}
