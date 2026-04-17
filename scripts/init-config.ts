// Standalone replacement for `anchor migrate` — initializes the singleton DcaConfig.
// Exists because anchor CLI 1.0's migrate harness conflicts with Yarn PnP CWD resolution.
// Idempotent: re-running after init is a no-op.
//
// Required env:
//   DEPLOYER_KEYPAIR_PATH       — signs initialize_config; becomes DcaConfig.admin
//   HOLDERSCAN_KEEPER_PUBKEY    — authorized to call execute_cycle
//   HOLDERSCAN_FEE_VAULT_PUBKEY — wSOL token account receiving upfront fees
//
// Optional env:
//   RPC_URL                         — default http://127.0.0.1:8899
//   HOLDERSCAN_DEFAULT_FREQUENCY    — seconds between cycles (default 14400 = 4h)
//   HOLDERSCAN_DEFAULT_NUM_CYCLES   — cycles per order       (default 42 = 7d @ 4h)
//   HOLDERSCAN_MIN_TOTAL_IN_AMOUNT  — min DCA notional in lamports (default 0.5 SOL)

import * as anchor from "@anchor-lang/core";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import BN from "bn.js";
import * as fs from "fs";
import * as path from "path";
import idl from "../target/idl/holderscan_dca.json";

const LAMPORTS_PER_SOL = new BN(1_000_000_000);

// Flat fee: 0.45% or 0.01 SOL — whichever is higher. No tiers.
const FEE_BPS = 45; // 0.45%
const MIN_FEE_LAMPORTS = LAMPORTS_PER_SOL.divn(100); // 0.01 SOL

function loadKeypair(p: string): Keypair {
  const expanded = p.startsWith("~") ? path.join(process.env.HOME!, p.slice(1)) : p;
  return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync(expanded, "utf-8"))));
}

function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`Missing env var: ${name}`);
  return v;
}

async function main() {
  const rpcUrl = process.env.RPC_URL ?? "http://127.0.0.1:8899";
  const connection = new Connection(rpcUrl, "confirmed");

  const deployer = loadKeypair(requireEnv("DEPLOYER_KEYPAIR_PATH"));
  const keeper = new PublicKey(requireEnv("HOLDERSCAN_KEEPER_PUBKEY"));
  const feeVault = new PublicKey(requireEnv("HOLDERSCAN_FEE_VAULT_PUBKEY"));
  const defaultCycleFrequency = new BN(process.env.HOLDERSCAN_DEFAULT_FREQUENCY ?? "14400");
  const defaultNumCycles = new BN(process.env.HOLDERSCAN_DEFAULT_NUM_CYCLES ?? "42");
  const minTotalInAmount = new BN(
    process.env.HOLDERSCAN_MIN_TOTAL_IN_AMOUNT ?? LAMPORTS_PER_SOL.divn(2).toString() // 0.5 SOL
  );

  const provider = new anchor.AnchorProvider(connection, new anchor.Wallet(deployer), {
    commitment: "confirmed",
  });
  anchor.setProvider(provider);
  const program = new anchor.Program(idl as any, provider);

  const [configPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("dca_config")],
    program.programId
  );

  const existing = await connection.getAccountInfo(configPda);
  if (existing) {
    console.log(`DcaConfig already initialized at ${configPda.toBase58()} — skipping.`);
    return;
  }

  console.log(`Initializing DcaConfig at ${configPda.toBase58()}`);
  console.log(`  admin       : ${deployer.publicKey.toBase58()}`);
  console.log(`  keeper      : ${keeper.toBase58()}`);
  console.log(`  fee_vault   : ${feeVault.toBase58()}`);
  console.log(`  fee_bps     : ${FEE_BPS} (${FEE_BPS / 100}%)`);
  console.log(`  min_fee     : ${MIN_FEE_LAMPORTS.toString()} lamports`);
  console.log(`  frequency   : ${defaultCycleFrequency.toString()}s`);
  console.log(`  num_cycles  : ${defaultNumCycles.toString()}`);
  console.log(`  min_total   : ${minTotalInAmount.toString()} lamports`);

  const sig = await program.methods
    .initializeConfig(
      feeVault,
      keeper,
      FEE_BPS,
      MIN_FEE_LAMPORTS,
      defaultCycleFrequency,
      defaultNumCycles,
      minTotalInAmount
    )
    .accounts({
      admin: deployer.publicKey,
      config: configPda,
    })
    .signers([deployer])
    .rpc();

  console.log(`Initialized. tx: ${sig}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
