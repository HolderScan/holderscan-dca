# HolderScan DCA

Solana program powering [HolderScan](https://holderscan.com) Dollar-Cost Average (DCA) orders. Users lock wSOL into an on-chain escrow and receive any SPL or Token-2022 output mint in equal slices on a fixed schedule. Orders sharing the same `(input_mint, output_mint)` pair at the same cycle boundary are aggregated into a single swap and distributed pro-rata — tighter routing than running each user's cycle as an isolated swap.

- **Program ID**: `2k7JFjY617MMCsshPMpRkYxR4Cx1gALPeFgNpfvCg4G5`
- **Network**: Solana mainnet-beta
- **Verified build**: see [Verified build](#verified-build) below. Canonical disclosure policy in the on-chain [`security.txt`](./programs/holderscan-dca/src/lib.rs).

## Overview

A DCA order has three parties:

- **Owner** — the end user. Signs `create_order`, funds the escrow, and can `cancel_order` at any time to recover unspent wSOL.
- **Keeper** — the HolderScan-operated off-chain service. Sole authorized signer of `execute_cycle` and `refund_cycle`. Runs the swap + delivery pipeline every cycle.
- **Admin** — HolderScan-controlled key. Holds `update_config` authority (fee parameters, schedule defaults, pause flag). Transfer is two-step.

The program itself is intentionally small: it enforces the schedule, debits wSOL from escrow on each cycle boundary, and exposes a refund hatch for failed swaps. It does **not** CPI into any DEX — routing and execution live off-chain.

## How execution works

Cycle execution runs off-chain in the HolderScan keeper service, which is the only principal authorized to call `execute_cycle` and `refund_cycle`. User funds live in per-order escrow PDAs between cycles; wSOL transits a keeper ATA only for the window needed to route a swap, and output tokens are delivered directly to the order owner's ATA.

### Cycle lifecycle

1. **Schedule.** The keeper polls on a fixed cadence (every 4 hours, aligned to UTC boundaries) and selects orders whose `next_cycle_at` has passed. The program — not the keeper — is the permission gate: `execute_cycle` rejects any call before `next_cycle_at` on-chain (`CycleTooEarly`).
2. **Batch.** Orders sharing the same `(input_mint, output_mint)` pair at the same poll boundary are grouped. One aggregate swap is routed for the batch's combined notional and the resulting output is distributed **pro-rata** to each order by its per-cycle input amount. Orders on different pairs — or the same pair at different cycles — execute independently.
3. **Drain.** The keeper calls `execute_cycle` for each order in the batch, moving wSOL from the escrow PDAs to the keeper's wSOL ATA. `cycles_remaining` and `next_cycle_at` are updated atomically with the transfer.
4. **Swap.** The keeper obtains routing from Jupiter and submits the swap. Large-notional batches may be split across multiple sub-swaps for liquidity reasons; each sub-swap's fill contributes to the pro-rata distribution.
5. **Deliver.** Output tokens are SPL-transferred directly from the keeper's ATA to each order owner's ATA. The program is not involved in delivery.

### Failure handling

The swap step can fail or partially fail after the drain has already landed. The keeper unwinds automatically as part of the same tick:

- **Total swap failure** — Keeper calls `refund_cycle` for every order in the batch. Each owner's wSOL is returned, one cycle is re-credited, and `next_cycle_at` is wound back one frequency step. For an order whose final cycle drained and closed the account, `refund_cycle` is not callable on a closed order; the keeper performs a direct SPL refund of the drained wSOL to the owner instead.
- **Partial swap success** — Owners receive pro-rata output for the filled portion and a pro-rata wSOL rebate (direct SPL transfer) for the unfilled portion. Cycles are not re-credited; the batch-level drain is treated as fulfilled.

No owner action is required to recover funds from a failed cycle.

### What the owner retains

- `cancel_order` is owner-signed and is not gated by the keeper, the admin, or the `paused` flag. Remaining escrow is always recoverable directly on-chain.
- `refund_cycle` cannot inflate `cycles_remaining` beyond the owner's originally-signed `initial_num_cycles`.
- The keeper cannot create orders, cannot alter an order's schedule or owner, and can only close an order as the natural consequence of its final cycle landing. Its only state-changing actions are a cycle debit (via `execute_cycle`) and its inverse refund (via `refund_cycle`).

### Token-2022 output

Output mints may be classic SPL Token or Token-2022. The program records the mint and emits events but never CPIs the output token program. Input mint is restricted to wSOL on-chain.

The keeper applies a strict acceptance policy to Token-2022 output mints — only mints whose transfer-affecting properties are immutable at mint-init are executed against:

- `mint_authority` revoked (supply is fixed; no dilution risk)
- `freeze_authority` revoked (user and keeper ATAs cannot be frozen mid-DCA)
- Extensions limited to `MetadataPointer` and `TokenMetadata` (cosmetic only)

Mints with any other extension — `TransferFee`, `TransferHook`, `DefaultAccountState`, `MintCloseAuthority`, `NonTransferable`, etc. — are rejected at pickup time and their cycles will not execute. Classic SPL Token mints are accepted without extension checks. Owners whose orders reference a non-conforming mint retain `cancel_order` and can recover remaining escrow at any time.

The acceptance filter runs at pickup, before any cycle's drain. Non-conforming mints never reach the swap or delivery step, so there is no class of "swap succeeded, delivery failed" failures caused by a Token-2022 extension the program didn't reject up front.

## Accounts

### `DcaConfig` (singleton)

PDA seeds: `["dca_config"]`. Initialized once by `initialize_config`; all subsequent mutations go through `update_config`.

| Field | Type | Notes |
|---|---|---|
| `admin` | `Pubkey` | Authority for `update_config`. Transfer via propose/accept. |
| `pending_admin` | `Option<Pubkey>` | Set by `propose_admin`, consumed by `accept_admin`. |
| `fee_vault` | `Pubkey` | wSOL token account that receives upfront fees. Verified to be a wSOL `TokenAccount` at `create_order` time. |
| `keeper` | `Pubkey` | Sole authorized signer of `execute_cycle` and `refund_cycle`. |
| `fee_bps` | `u16` | Percentage fee in bps. Capped at `MAX_FEE_BPS = 300` (3%). |
| `min_fee_lamports` | `u64` | Absolute fee floor in wSOL lamports. Capped at `MAX_MIN_FEE_LAMPORTS = 1 SOL`. |
| `default_cycle_frequency` | `i64` | Seconds between cycles. Bounded `[60, 30 days]`. |
| `default_num_cycles` | `u64` | Cycle count. Capped at `MAX_NUM_CYCLES = 1000` — at the current 4h cadence, ~166 days of order duration. |
| `min_total_in_amount` | `u64` | Minimum input amount (wSOL lamports) the user must commit to open an order — applied to the gross input, since the fee is deducted from it. Must be ≥ `default_num_cycles` so the post-fee per-cycle slice is ≥ 1 lamport in normal configurations. |
| `paused` | `bool` | Kill-switch on `create_order` and `execute_cycle`. `cancel_order` stays available. |

### `DcaOrder` (per-order)

PDA seeds: `["dca_order", owner, input_mint, output_mint, created_at.to_le_bytes()]`. `created_at` is an `i64` instruction argument supplied at creation time, bounded to ±60s of on-chain time (`CREATED_AT_TOLERANCE_SECS`); it is used only for PDA derivation and is not stored on the account.

| Field | Type | Notes |
|---|---|---|
| `owner` | `Pubkey` | |
| `input_mint` | `Pubkey` | Always wSOL (enforced at `create_order`). |
| `output_mint` | `Pubkey` | Any SPL Token or Token-2022 mint. |
| `in_amount_per_cycle` | `u64` | `floor((total_in_amount - fee) / num_cycles)`. Fee is taken out of the input upfront; remainder drained to owner on the final cycle. |
| `cycles_remaining` | `u64` | |
| `initial_num_cycles` | `u64` | Immutable snapshot at creation. Caps `refund_cycle`. |
| `cycle_frequency` | `i64` | Snapshot of `config.default_cycle_frequency` at creation time. |
| `next_cycle_at` | `i64` | Unix timestamp. |
| `is_active` | `bool` | |

Escrow token account PDA: `["escrow", order]`. Escrow authority PDA: `["escrow_auth", order]`.

## Instructions

| Instruction | Signer | Effect |
|---|---|---|
| `initialize_config` | initial admin | One-shot config init. |
| `update_config` | admin | Patch any subset of config fields (within caps); pause/unpause. |
| `propose_admin` | current admin | Set `pending_admin`. |
| `accept_admin` | proposed admin | Complete two-step transfer. |
| `create_order` | owner | Pull `total_in_amount` wSOL from the owner: fee goes to vault, the remainder funds the escrow, and the order is opened. |
| `execute_cycle` | keeper | Debit `in_amount_per_cycle` from escrow to keeper's wSOL ATA; decrement `cycles_remaining`; close on final cycle. |
| `refund_cycle` | keeper | Undo the most recent cycle if the off-chain swap failed to land. |
| `cancel_order` | owner | Refund remaining escrow, close the order account. |

## Fee model

Fees are taken **upfront at `create_order`** and **deducted from the user's input** — the user signs for one number (`total_in_amount`) and pays exactly that. The fee goes to `config.fee_vault`; the remainder funds the DCA escrow. The program contains no fee-refund path: fees remain in `config.fee_vault` across every subsequent outcome — `cancel_order`, `refund_cycle`, partial swap success, total swap failure, and any keeper-side downtime that prevents execution. Any remediation for those cases is a HolderScan operational matter and is not enforced on-chain. Fee amount:

    fee = max(total_in_amount * fee_bps / 10_000, min_fee_lamports)
    escrow = total_in_amount - fee

Current mainnet parameters: **45 bps (0.45%)** with a **0.01 SOL floor**

The floor exists because HolderScan commits to executing orders even if the percentage fee on a small order would not cover transaction + priority-fee costs.

## Security model

The program's security rests on a single invariant: **the owner retains ultimate authority over their escrowed funds.** The admin's power is bounded to protocol parameters within compile-time caps; the keeper's power is scoped to executing a single cycle's worth of wSOL on schedule. Neither can reach an owner's funds outside that narrow, sanctioned path.

- **Owners hold permanent cancel authority.** `cancel_order` is owner-signed, not gated by the keeper, admin, or `paused` flag, and returns any remaining escrow — directly on-chain, no intermediary.
- **Admin authority is narrow.** `update_config` adjusts fees, schedule defaults, and the pause flag, all within compile-time caps: 3% max fee, 1 SOL fee floor cap, 30-day max cycle frequency, 1000 cycles per order. The admin has no path to user escrow, to existing orders, or to overriding `cancel_order`. Admin transfer is two-step (`propose_admin` / `accept_admin`) so a mistyped pubkey cannot lock the protocol.
- **Keeper authority is scoped to two sanctioned state transitions:** `execute_cycle` (debit one cycle's wSOL once `next_cycle_at` has passed) and `refund_cycle` (return that amount and re-credit). The program enforces every other limit on-chain — no schedule changes, no owner changes, no escrow access outside the cycle window (`CycleTooEarly`), no early closes, no config changes, no inflating `cycles_remaining` beyond `initial_num_cycles` (`CycleOverRefund`).
- **Pause preserves owner exits.** `paused` halts new order creation and cycle execution but leaves `cancel_order` and `refund_cycle` open — pause cannot trap user funds.
- **On-chain bounds on user input.** `created_at` is constrained to a ±60s window around on-chain time (blocks PDA-seed grinding); input mint is restricted to wSOL so fee math and cycle accounting always operate in lamport semantics.
- **Final-cycle edge case.** `execute_cycle` closes the escrow and order accounts when the last cycle lands, so `refund_cycle` is not reachable for a post-close failure. In that narrow window the keeper returns any drained wSOL via a direct SPL transfer to the owner — the one point where the protocol depends on keeper behavior rather than on-chain enforcement.

See [`programs/holderscan-dca/src/lib.rs`](./programs/holderscan-dca/src/lib.rs) for the on-chain `security.txt` and the canonical disclosure policy.

## Build and test

### Rust / Anchor

    cargo build
    cargo test         # litesvm integration tests in programs/holderscan-dca/tests
    anchor build       # produces target/idl/holderscan_dca.json and the .so

Rust toolchain is pinned via [`rust-toolchain.toml`](./rust-toolchain.toml) to Rust 1.89.0.

### TypeScript

Requires a running localnet (`solana-test-validator`) or [surfpool](https://github.com/txtx/surfpool).

    yarn install
    yarn smoke         # minimal end-to-end
    yarn place-orders  # N fresh wallets, one order each
    yarn init-config   # one-shot config initialization (mainnet/devnet)

Environment variables read by the scripts are listed at the top of each file under `scripts/` and `tests/`.

## Verified build

Reproducible builds let any third party confirm that the bytecode currently deployed at the Program ID above was produced from the source in this repo — no trust in HolderScan's build or deploy pipeline required. The mainnet deployment is reproducible from this repo at the `v0.2.0` tag. To verify locally:

    cargo install solana-verify

    # Build in the canonical Docker image
    solana-verify build --library-name holderscan_dca

    # Compare hashes
    solana-verify get-executable-hash target/deploy/holderscan_dca.so
    solana-verify get-program-hash 2k7JFjY617MMCsshPMpRkYxR4Cx1gALPeFgNpfvCg4G5 -u mainnet-beta

After verification is submitted (`solana-verify verify-from-repo --remote ...`), the record is written to an on-chain PDA and indexed by OtterSec's verified-builds registry; explorers like Solscan and SolanaFM display the "verified build" badge once it's live.

## Reporting vulnerabilities

Email **contact@holderscan.com**. We commit to a 90-day coordinated-disclosure window and offer safe harbor for good-faith security research. Please avoid testing on mainnet with non-trivial amounts.

## License

See [LICENSE](./LICENSE).
