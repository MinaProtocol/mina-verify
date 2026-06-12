# Using mina-verify as a backend (trustless / verify-before-ingest)

`mina-verify` verifies a Mina block's Pickles/kimchi SNARK proof. If a block
verifies, it — and its whole ancestry, by recursion — is valid, with **no trust in
whoever supplied it**. An indexer or service built on it can't be poisoned by a
lying or compromised node: every persisted row is proof-backed.

## The pattern

```
block bytes (untrusted) → decode → VERIFY proof → ingest only if valid → persist
                                       │
                                       └─ invalid → reject, never touch the store
```

## Core loop

```rust
use mina_verify::{block_from_gossip_payload, ChainMonitor, Ingest, Verifier};
use mina_verify_capture::{network_seeds, subscribe_blocks};
use std::{ops::ControlFlow, sync::mpsc};

let verifier = Verifier::for_network("devnet")?;   // or with_index_json(..) for mainnet/mesa-mut
let mut monitor = ChainMonitor::new(512);
let (tx, rx) = mpsc::channel::<Vec<u8>>();

// Verification is multi-second CPU — run it OFF the gossip loop (worker thread/pool).
std::thread::spawn(move || {
    while let Ok(payload) = rx.recv() {
        let Ok(block) = block_from_gossip_payload(&payload) else { continue };
        match verifier.verify_tip(block) {                  // ← verify-before-ingest
            Ok(Some(tip)) => match monitor.ingest(&tip) {
                Ingest::Genesis | Ingest::Extend { .. } => persist_canonical(&tip),
                Ingest::Reorg { common_ancestor, depth, .. } => {
                    rollback_to(common_ancestor);           // ← critical for an indexer
                    persist_canonical(&tip);
                }
                Ingest::Fork { .. } => persist_side_branch(&tip), // not canonical
                Ingest::Behind { .. } | Ingest::Duplicate | Ingest::Unlinked => {}
            },
            Ok(None) => { /* invalid proof — REJECT, do not persist */ }
            Err(_)   => { /* malformed block field */ }
        }
    }
});

let (chain_id, peers) = network_seeds("devnet").unwrap();
subscribe_blocks(chain_id, peers, None, |payload| {
    let _ = tx.send(payload.to_vec());                      // hand off instantly
    ControlFlow::Continue(())
}).await;
```

## API cheat-sheet

- `Verifier::for_network("devnet"|"mainnet") -> Result<_, VerifierError>` ·
  `with_index_json(&str)` (any network) · `verify_block(&Block) -> bool` ·
  `verify_header(&BlockHeader) -> bool` · `verify_tip(Block) -> Result<Option<VerifiedTip>, _>`
- `block_from_gossip_payload(&[u8])` (gossip wire) · `block_from_binprot(&[u8])`
  (raw block, e.g. from RPC/archive) → `Block`
- `VerifiedTip`: `.block()` `.state_hash()` `.consensus_state()` `.height()`
- `ChainMonitor::ingest(&tip) -> Ingest`
  (`Genesis / Extend / Reorg{common_ancestor,depth} / Fork / Behind / Duplicate / Unlinked`),
  `.best()`, `.best_height()`
- Re-exports (so you don't need `mina-p2p-messages`): `Block`, `BlockHeader`, `StateHash`

## What's safe to index from a verified block

Its contents are committed in the verified state: `block.body.staged_ledger_diff`
(the transactions), the protocol/consensus state, the ledger hashes. **Not**
verifiable: zkApp *events* (no on-chain commitment) — treat those as trusted-source
data.

## Block source

- **Live tip** → gossip (`subscribe_blocks`). New blocks only.
- **Historical backfill** → mina-verify does *not* fetch history. Feed it blocks from
  your own source (archive / precomputed / RPC) via `block_from_binprot` and verify
  each. (Recursion: verifying the tip already attests history, so per-block
  re-verification of the past is optional.)

### Unified `BlockSource` (`mina-verify-monitor`)

When you just want *one verified block from somewhere*, `mina-verify-monitor` wraps
acquisition + verification behind a single call — the shape the TS SDK / MCP binds to:

```rust
use mina_verify::Verifier;
use mina_verify_monitor::{verify_from, BlockSource};

let verifier = Verifier::for_network("devnet")?;

// (1) a precomputed block (GCS / archive JSON) — no networking
let facts = verify_from(&verifier, BlockSource::Precomputed(json)).await?;

// (2) a live node's best tip over libp2p RPC (the seed path; no gossip-mesh wait)
let tip = verify_from(&verifier, BlockSource::node("devnet")).await?;
// -> VerifiedBlock { height, state_hash, previous_state_hash, staged_ledger_hash }
```

Both return proof-backed [`VerifiedBlock`] facts; any `Err` (bad proof, unknown
network, RPC timeout) means *don't ingest*. The node arm is the basis of an **endpoint
honesty check**: verify the live tip, then compare its `state_hash` / ledger hash to
what an untrusted GraphQL endpoint claims — a mismatch proves the endpoint is lying.

Examples: `cargo run --example ingest -p mina-verify-monitor -- node devnet` ·
`-- precomputed block.json` · `to_precomputed` converts a captured gossip block to the
precomputed JSON form.

## Build / dependency gotchas (read this or it won't compile)

- Toolchain **Rust 1.94** (`rust-toolchain.toml`); build needs `libssl-dev` / `pkg-config`.
- `[patch]` does **not** propagate from a git dependency — replicate at your workspace root:
  ```toml
  [patch.crates-io]
  num-bigint   = { git = "https://github.com/openmina/num-bigint",   branch = "rebase-onstack" }
  num-rational = { git = "https://github.com/openmina/num-rational", branch = "rebase-onstack" }
  ```
- A transitive dep (`multihash → core2 0.4.0`) is **yanked**; a fresh resolve fails.
  Reuse `mina-verify`'s `Cargo.lock` pins. **Lowest-friction: add your adapter as a
  crate inside the `mina-verify` workspace** (inherits lock + patch); otherwise vendor
  the lock.
- Networks: **devnet works out of the box.** mainnet's embedded VK is stale-format and
  mesa-mut has none → supply a verifier-index JSON via `Verifier::with_index_json`.

## Data sourcing (GraphQL / RPC)

- `mina-verify` uses **no GraphQL** — blocks come from libp2p gossip
  (`mina-verify-capture`).
- An HTTP GraphQL client (e.g. `mina-sdk-rust`) is fine for the **untrusted bulk-data
  tier** (accounts, history, tx search) — data you don't verify or cross-check against
  a verified root. Not a source for verified blocks.
- Mina RPC (`get_best_tip`, `get_transition_chain`) is a **libp2p** substream
  (`coda/rpcs/0.0.1`), not HTTP — it belongs in `mina-verify-capture` (which has the
  transport), not in a GraphQL client. (Roadmap item "B".)
