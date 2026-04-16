// End-to-end smoke test for HolderScan DCA.
//
// Flow: fresh user → wrap SOL → create_order (short frequency) → keeper runs
// every cycle → final cycle auto-closes the order → verify keeper received the
// total notional and order account is gone. Also tests cancel on a second order.
//
// Assumes: `anchor deploy` and `anchor migrate` have already run, so the
// program is live and DcaConfig is initialized.
//
// Required env:
//   DEPLOYER_KEYPAIR_PATH      — used as AnchorProvider wallet (pays nothing here, just for RPC context)
//   KEEPER_KEYPAIR_PATH        — signs execute_cycle; must match DcaConfig.keeper
//   HOLDERSCAN_FEE_VAULT_PUBKEY — WSOL ATA from `spl-token create-account`
//
// Optional env:
//   RPC_URL          — default https://api.devnet.solana.com
//   OUTPUT_MINT      — any existing SPL Mint; default USDC devnet
//   TEST_TOTAL_SOL   — notional per test order, default 0.04
//
// Note: num_cycles and cycle_frequency are no longer per-order overrides —
// they come from DcaConfig's defaults. Pick a config with a short schedule
// (e.g. 60s x 4 cycles) before running this test.

import * as anchor from "@anchor-lang/core";
import {
  Connection,
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
  Transaction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import {
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountIdempotentInstruction,
  createSyncNativeInstruction,
  getAccount,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import BN from "bn.js";
import * as fs from "fs";
import * as path from "path";
import idl from "../target/idl/holderscan_dca.json";

const DEVNET_USDC = new PublicKey("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");

function loadKeypair(p: string): Keypair {
  const expanded = p.startsWith("~") ? path.join(process.env.HOME!, p.slice(1)) : p;
  const raw = JSON.parse(fs.readFileSync(expanded, "utf-8"));
  return Keypair.fromSecretKey(Uint8Array.from(raw));
}

function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`Missing env var: ${name}`);
  return v;
}

async function wrapSol(
  connection: Connection,
  owner: Keypair,
  amountLamports: number
): Promise<PublicKey> {
  const ata = getAssociatedTokenAddressSync(NATIVE_MINT, owner.publicKey);
  const tx = new Transaction().add(
    createAssociatedTokenAccountIdempotentInstruction(
      owner.publicKey,
      ata,
      owner.publicKey,
      NATIVE_MINT
    ),
    SystemProgram.transfer({
      fromPubkey: owner.publicKey,
      toPubkey: ata,
      lamports: amountLamports,
    }),
    createSyncNativeInstruction(ata)
  );
  await sendAndConfirmTransaction(connection, tx, [owner]);
  return ata;
}

async function ensureAta(
  connection: Connection,
  payer: Keypair,
  owner: PublicKey,
  mint: PublicKey
): Promise<PublicKey> {
  const ata = getAssociatedTokenAddressSync(mint, owner);
  const info = await connection.getAccountInfo(ata);
  if (info) return ata;
  const tx = new Transaction().add(
    createAssociatedTokenAccountIdempotentInstruction(payer.publicKey, ata, owner, mint)
  );
  await sendAndConfirmTransaction(connection, tx, [payer]);
  return ata;
}

async function airdrop(connection: Connection, to: PublicKey, sol: number) {
  const sig = await connection.requestAirdrop(to, sol * LAMPORTS_PER_SOL);
  await connection.confirmTransaction(sig, "confirmed");
}

async function main() {
  const rpcUrl = process.env.RPC_URL ?? "https://api.devnet.solana.com";
  const connection = new Connection(rpcUrl, "confirmed");

  const deployer = loadKeypair(requireEnv("DEPLOYER_KEYPAIR_PATH"));
  const keeper = loadKeypair(requireEnv("KEEPER_KEYPAIR_PATH"));
  const feeVault = new PublicKey(requireEnv("HOLDERSCAN_FEE_VAULT_PUBKEY"));
  const outputMint = process.env.OUTPUT_MINT
    ? new PublicKey(process.env.OUTPUT_MINT)
    : DEVNET_USDC;

  const totalSol = Number(process.env.TEST_TOTAL_SOL ?? "0.04");

  const provider = new anchor.AnchorProvider(
    connection,
    new anchor.Wallet(deployer),
    { commitment: "confirmed" }
  );
  anchor.setProvider(provider);
  const program = new anchor.Program(idl as any, provider);

  const [configPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("dca_config")],
    program.programId
  );
  const configAccount = await connection.getAccountInfo(configPda);
  if (!configAccount) {
    throw new Error("DcaConfig not initialized — run init-config.ts first.");
  }
  const config: any = await (program.account as any).dcaConfig.fetch(configPda);
  const numCycles: number = Number(config.defaultNumCycles);
  const frequency: number = Number(config.defaultCycleFrequency);

  console.log(`network:    ${rpcUrl}`);
  console.log(`program:    ${program.programId.toBase58()}`);
  console.log(`config:     ${configPda.toBase58()}`);
  console.log(`keeper:     ${keeper.publicKey.toBase58()}`);
  console.log(`fee_vault:  ${feeVault.toBase58()}`);
  console.log(`output:     ${outputMint.toBase58()}`);
  console.log(`schedule:   ${numCycles} cycles @ ${frequency}s (from config)`);
  console.log(`notional:   ${totalSol} SOL`);
  console.log();

  // Fresh user per run — avoids PDA collisions on re-runs
  const user = Keypair.generate();
  console.log(`user:       ${user.publicKey.toBase58()}`);
  await airdrop(connection, user.publicKey, 1);

  const totalLamports = Math.floor(totalSol * LAMPORTS_PER_SOL);
  // Notional must be divisible by num_cycles
  const perCycle = Math.floor(totalLamports / numCycles);
  const roundedTotal = perCycle * numCycles;
  // Fund extra for upfront fee (up to 45 bps) + rent slack
  await wrapSol(connection, user, roundedTotal + Math.ceil(roundedTotal * 0.005) + 10_000);
  const userWsolAta = getAssociatedTokenAddressSync(NATIVE_MINT, user.publicKey);
  console.log(`user WSOL:  ${userWsolAta.toBase58()}`);

  // Keeper needs a WSOL ATA to receive each cycle's input
  const keeperWsolAta = await ensureAta(connection, keeper, keeper.publicKey, NATIVE_MINT);
  console.log(`keeper WSOL:${keeperWsolAta.toBase58()}`);
  console.log();

  // ── Test 1: full lifecycle (execute all cycles, auto-close) ──────────────
  {
    const createdAt = new BN(Math.floor(Date.now() / 1000));
    const [orderPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("dca_order"),
        user.publicKey.toBuffer(),
        NATIVE_MINT.toBuffer(),
        outputMint.toBuffer(),
        createdAt.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );
    const [escrowPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("escrow"), orderPda.toBuffer()],
      program.programId
    );
    const [escrowAuthority] = PublicKey.findProgramAddressSync(
      [Buffer.from("escrow_auth"), orderPda.toBuffer()],
      program.programId
    );

    console.log(`[test 1] create order (${orderPda.toBase58()})`);
    const sig = await program.methods
      .createOrder(new BN(roundedTotal), createdAt)
      .accounts({
        owner: user.publicKey,
        config: configPda,
        inputMint: NATIVE_MINT,
        outputMint,
        order: orderPda,
        escrowTokenAccount: escrowPda,
        escrowAuthority,
        userInputAta: userWsolAta,
        feeVault,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([user])
      .rpc();
    console.log(`         tx: ${sig}`);

    const keeperStart = BigInt((await getAccount(connection, keeperWsolAta)).amount);

    for (let i = 1; i <= numCycles; i++) {
      const order: any = await (program.account as any).dcaOrder.fetch(orderPda);
      const nextCycleMs = order.nextCycleAt.toNumber() * 1000;
      const waitMs = nextCycleMs - Date.now() + 2000;
      if (waitMs > 0) {
        console.log(`[test 1] wait ${Math.ceil(waitMs / 1000)}s for cycle ${i}`);
        await new Promise((r) => setTimeout(r, waitMs));
      }

      const sig = await program.methods
        .executeCycle()
        .accounts({
          keeper: keeper.publicKey,
          config: configPda,
          order: orderPda,
          owner: user.publicKey,
          escrowTokenAccount: escrowPda,
          escrowAuthority,
          userInputAta: userWsolAta,
          keeperInputAta: keeperWsolAta,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([keeper])
        .rpc();
      console.log(`[test 1] cycle ${i}/${numCycles} tx: ${sig}`);
    }

    const keeperEnd = BigInt((await getAccount(connection, keeperWsolAta)).amount);
    const received = keeperEnd - keeperStart;
    if (received !== BigInt(roundedTotal)) {
      throw new Error(`keeper received ${received}, expected ${roundedTotal}`);
    }

    const finalOrder = await (program.account as any).dcaOrder.fetchNullable(orderPda);
    if (finalOrder !== null) throw new Error("order should be closed after final cycle");

    console.log(`[test 1] ✓ all ${numCycles} cycles executed; order closed; keeper received ${received}`);
    console.log();
  }

  // ── Test 2: cancel after first cycle ─────────────────────────────────────
  {
    // Top up user so second order has funds
    await wrapSol(connection, user, roundedTotal + Math.ceil(roundedTotal * 0.005) + 10_000);

    const createdAt = new BN(Math.floor(Date.now() / 1000));
    const [orderPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("dca_order"),
        user.publicKey.toBuffer(),
        NATIVE_MINT.toBuffer(),
        outputMint.toBuffer(),
        createdAt.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );
    const [escrowPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("escrow"), orderPda.toBuffer()],
      program.programId
    );
    const [escrowAuthority] = PublicKey.findProgramAddressSync(
      [Buffer.from("escrow_auth"), orderPda.toBuffer()],
      program.programId
    );

    console.log(`[test 2] create order then cancel (${orderPda.toBase58()})`);
    await program.methods
      .createOrder(new BN(roundedTotal), createdAt)
      .accounts({
        owner: user.publicKey,
        config: configPda,
        inputMint: NATIVE_MINT,
        outputMint,
        order: orderPda,
        escrowTokenAccount: escrowPda,
        escrowAuthority,
        userInputAta: userWsolAta,
        feeVault,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([user])
      .rpc();

    // Execute exactly one cycle
    await program.methods
      .executeCycle()
      .accounts({
        keeper: keeper.publicKey,
        config: configPda,
        order: orderPda,
        owner: user.publicKey,
        escrowTokenAccount: escrowPda,
        escrowAuthority,
        userInputAta: userWsolAta,
        keeperInputAta: keeperWsolAta,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([keeper])
      .rpc();

    const refundBefore = BigInt((await getAccount(connection, userWsolAta)).amount);
    const cancelSig = await program.methods
      .cancelOrder()
      .accounts({
        owner: user.publicKey,
        order: orderPda,
        escrowTokenAccount: escrowPda,
        escrowAuthority,
        userInputAta: userWsolAta,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([user])
      .rpc();
    console.log(`[test 2] cancel tx: ${cancelSig}`);

    const refundAfter = BigInt((await getAccount(connection, userWsolAta)).amount);
    const refunded = refundAfter - refundBefore;
    const expected = BigInt(roundedTotal - perCycle);
    if (refunded !== expected) {
      throw new Error(`refund ${refunded}, expected ${expected}`);
    }

    const finalOrder = await (program.account as any).dcaOrder.fetchNullable(orderPda);
    if (finalOrder !== null) throw new Error("cancelled order should be closed");

    console.log(`[test 2] ✓ cancelled mid-flight; user refunded ${refunded}`);
  }

  console.log("\nAll smoke checks passed.");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
