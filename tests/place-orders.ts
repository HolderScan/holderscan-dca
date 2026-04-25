// Localnet end-to-end test: spawns N fresh wallets, each places one DCA order
// per test token, then polls until the externally-running keeper closes them all.
//
// Assumes:
//   - solana-test-validator running with --clone for the output mints
//   - program deployed and DcaConfig initialized
//   - keeper service running (polling ~60s)
//
// Required env:
//   DEPLOYER_KEYPAIR_PATH  — funds airdrops / acts as provider wallet
//
// Optional env:
//   RPC_URL                — default http://127.0.0.1:8899
//   NUM_WALLETS            — default 3
//   SOL_PER_ORDER          — default 0.06 (must divide evenly by config.default_num_cycles)
//   TIMEOUT_S              — default 540 (9 min — leaves 1 min headroom under 10)
//
// Note: num_cycles and cycle_frequency come from DcaConfig and are not
// configurable per-order. Use init-config.ts with a short schedule for tests.

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
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import BN from "bn.js";
import * as fs from "fs";
import * as path from "path";
import idl from "../target/idl/holderscan_dca.json";

// Cm6fNnMk...pump is Token-2022; excluded until program supports Token-2022 mints.
const OUTPUT_MINTS = [
  new PublicKey("Ce2gx9KGXJ6C9Mp5b5x1sn9Mg87JwEbrQby4Zqo3pump"),
  new PublicKey("H74CYmXgMkYHYuSRsZt6RJb4NYp2u72Vw8BS5huApump"),
];

function loadKeypair(p: string): Keypair {
  const expanded = p.startsWith("~") ? path.join(process.env.HOME!, p.slice(1)) : p;
  return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync(expanded, "utf-8"))));
}

function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`Missing env var: ${name}`);
  return v;
}

async function airdrop(connection: Connection, to: PublicKey, sol: number) {
  const sig = await connection.requestAirdrop(to, sol * LAMPORTS_PER_SOL);
  await connection.confirmTransaction(sig, "confirmed");
}

async function wrapSol(connection: Connection, owner: Keypair, amountLamports: number) {
  const ata = getAssociatedTokenAddressSync(NATIVE_MINT, owner.publicKey);
  const tx = new Transaction().add(
    createAssociatedTokenAccountIdempotentInstruction(owner.publicKey, ata, owner.publicKey, NATIVE_MINT),
    SystemProgram.transfer({ fromPubkey: owner.publicKey, toPubkey: ata, lamports: amountLamports }),
    createSyncNativeInstruction(ata)
  );
  await sendAndConfirmTransaction(connection, tx, [owner]);
  return ata;
}

type OrderHandle = {
  label: string;
  owner: PublicKey;
  outputMint: PublicKey;
  orderPda: PublicKey;
  createdAt: number;
  perCycle: bigint;
  totalCycles: number;
};

async function main() {
  const rpcUrl = process.env.RPC_URL ?? "http://127.0.0.1:8899";
  const connection = new Connection(rpcUrl, "confirmed");

  const deployer = loadKeypair(requireEnv("DEPLOYER_KEYPAIR_PATH"));

  const numWallets = Number(process.env.NUM_WALLETS ?? "3");
  const solPerOrder = Number(process.env.SOL_PER_ORDER ?? "0.06");
  const timeoutS = Number(process.env.TIMEOUT_S ?? "540");

  const provider = new anchor.AnchorProvider(connection, new anchor.Wallet(deployer), {
    commitment: "confirmed",
  });
  anchor.setProvider(provider);
  const program = new anchor.Program(idl as any, provider);

  const [configPda] = PublicKey.findProgramAddressSync([Buffer.from("dca_config")], program.programId);
  const configInfo = await connection.getAccountInfo(configPda);
  if (!configInfo) throw new Error("DcaConfig not initialized — run init-config.ts first");
  const config: any = await (program.account as any).dcaConfig.fetch(configPda);
  const feeVault: PublicKey = config.feeVault;
  const numCycles: number = Number(config.defaultNumCycles);
  const frequency: number = Number(config.defaultCycleFrequency);

  const lamportsPerOrder = Math.floor(solPerOrder * LAMPORTS_PER_SOL);
  if (lamportsPerOrder % numCycles !== 0) {
    throw new Error(
      `SOL_PER_ORDER lamports (${lamportsPerOrder}) must be divisible by config.defaultNumCycles (${numCycles})`
    );
  }
  if (lamportsPerOrder < Number(config.minTotalInAmount)) {
    throw new Error(
      `SOL_PER_ORDER (${lamportsPerOrder} lamports) is below config.minTotalInAmount (${config.minTotalInAmount.toString()})`
    );
  }
  const perCycle = lamportsPerOrder / numCycles;

  const totalOrders = numWallets * OUTPUT_MINTS.length;
  const totalSol = solPerOrder * totalOrders;

  console.log(`rpc:         ${rpcUrl}`);
  console.log(`program:     ${program.programId.toBase58()}`);
  console.log(`config:      ${configPda.toBase58()}`);
  console.log(`fee_vault:   ${feeVault.toBase58()}`);
  console.log(`wallets:     ${numWallets}`);
  console.log(`tokens:      ${OUTPUT_MINTS.length}`);
  console.log(`orders:      ${totalOrders} (${solPerOrder} SOL × ${numCycles} cycles @ ${frequency}s each)`);
  console.log(`total notl:  ${totalSol.toFixed(3)} SOL`);
  console.log(`timeout:     ${timeoutS}s`);
  console.log();

  // Create wallets + fund + wrap SOL
  const wallets: Keypair[] = [];
  for (let i = 0; i < numWallets; i++) {
    const w = Keypair.generate();
    wallets.push(w);
    // enough SOL for: rent for (order + escrow) × tokens, wSOL wrapping, tx fees.
    // The upfront fee comes out of `lamportsPerOrder` (inclusive), so no extra
    // funding is needed for it.
    await airdrop(connection, w.publicKey, 2);
    const neededWsol = lamportsPerOrder * OUTPUT_MINTS.length;
    await wrapSol(connection, w, neededWsol + 50_000);
    console.log(`wallet[${i}]:   ${w.publicKey.toBase58()}`);
  }
  console.log();

  // Place orders
  const handles: OrderHandle[] = [];
  const baseTs = Math.floor(Date.now() / 1000);
  let seq = 0;
  for (let w = 0; w < wallets.length; w++) {
    const user = wallets[w];
    const userWsol = getAssociatedTokenAddressSync(NATIVE_MINT, user.publicKey);
    for (const outputMint of OUTPUT_MINTS) {
      // Unique created_at per (wallet, mint) — seeds the order PDA
      const createdAt = baseTs + seq++;
      const createdAtBn = new BN(createdAt);
      const [orderPda] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("dca_order"),
          user.publicKey.toBuffer(),
          NATIVE_MINT.toBuffer(),
          outputMint.toBuffer(),
          createdAtBn.toArrayLike(Buffer, "le", 8),
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

      const label = `w${w}→${outputMint.toBase58().slice(0, 6)}`;
      const sig = await program.methods
        .createOrder(new BN(lamportsPerOrder), createdAtBn)
        .accounts({
          owner: user.publicKey,
          config: configPda,
          inputMint: NATIVE_MINT,
          outputMint,
          order: orderPda,
          escrowTokenAccount: escrowPda,
          escrowAuthority,
          userInputAta: userWsol,
          feeVault,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .signers([user])
        .rpc();
      console.log(`  [${label}] order=${orderPda.toBase58()} tx=${sig}`);

      handles.push({
        label,
        owner: user.publicKey,
        outputMint,
        orderPda,
        createdAt,
        perCycle: BigInt(perCycle),
        totalCycles: numCycles,
      });
    }
  }
  console.log();
  console.log(`Placed ${handles.length} orders. Waiting on keeper to drain them…`);
  console.log();

  // Poll until every order account is closed, or timeout
  const deadline = Date.now() + timeoutS * 1000;
  const seenCyclesRemaining = new Map<string, number>(); // orderPda -> last-observed cycles_remaining
  for (const h of handles) seenCyclesRemaining.set(h.orderPda.toBase58(), numCycles);
  const closedAt = new Map<string, number>();

  while (Date.now() < deadline) {
    let openCount = 0;
    for (const h of handles) {
      const key = h.orderPda.toBase58();
      if (closedAt.has(key)) continue;
      const acct = await (program.account as any).dcaOrder.fetchNullable(h.orderPda);
      if (acct === null) {
        closedAt.set(key, Date.now());
        console.log(`  [${h.label}] CLOSED`);
        continue;
      }
      openCount++;
      const remaining = Number(acct.cyclesRemaining);
      const last = seenCyclesRemaining.get(key)!;
      if (remaining !== last) {
        console.log(`  [${h.label}] cycles_remaining ${last} → ${remaining}`);
        seenCyclesRemaining.set(key, remaining);
      }
    }
    if (openCount === 0) break;
    await new Promise((r) => setTimeout(r, 5000));
  }

  // Summary
  const closed = closedAt.size;
  console.log();
  console.log(`=== Summary ===`);
  console.log(`closed:  ${closed} / ${handles.length}`);
  if (closed < handles.length) {
    console.log(`open orders (keeper did not fully drain within ${timeoutS}s):`);
    for (const h of handles) {
      const key = h.orderPda.toBase58();
      if (!closedAt.has(key)) {
        console.log(`  [${h.label}] remaining=${seenCyclesRemaining.get(key)} pda=${key}`);
      }
    }
    process.exit(1);
  }
  console.log(`All orders drained by keeper.`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
