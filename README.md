# mina-verify

Trustless verification of Mina blocks in Rust.

Point it at an **untrusted** source — a node, an indexer, or a raw gossip message —
and it verifies the block's Pickles/kimchi blockchain SNARK proof against the
network verification key. If the proof verifies, the entire chain history up to that
block is valid by Pickles recursion. No trust in the source is required.

This is the foundation of a client-side system-integrity monitor / light client.

## Crates

| Crate | What it is |
|-------|------------|
| [`mina-verify`](crates/mina-verify) | The verification library. `Verifier::devnet().verify_block(&block)` and gossip-payload decoding. Pure verification — the intended dependency for downstream consumers (CLI, mobile, indexer). |
| [`mina-verify-capture`](crates/mina-verify-capture) | Connects to the live devnet over Mina's libp2p (pnet keyed by chain_id), subscribes to the consensus-gossip topic, and saves a block. The trust input, obtained without any trusted endpoint. |
| [`mina-verify-cli`](crates/mina-verify-cli) | `mina-verify <gossip-payload-file>` — decode and verify a captured block. |

## Quick start

```sh
# 1. capture a live devnet block off the gossip network (~1-5 min)
cargo run -p mina-verify-capture          # writes captured/block-0.gossipbin

# 2. verify it
cargo run -p mina-verify-cli -- captured/block-0.gossipbin
# -> devnet block height 526706: verify_block = true
```

## How it works

The verification core (`verify_block`, the embedded verification key, the SRS) is
provided by OpenMina's `mina-tree` crate; `mina-verify` is a thin wrapper. The call
is `verify_block(&header, &BlockVerifier::make(), &get_srs::<Fp>())`. Only
`header.protocol_state` and `header.protocol_state_proof` participate.

## Networks

`Verifier::for_network("devnet"|"mainnet")` (or `MINA_NETWORK=…` for the binaries).

- **devnet** — fully working (verified live blocks + a real fork).
- **mainnet** — wired, but the mainnet blockchain verifier index embedded in
  `mina-rust@ab69eaed` is in a stale JSON format (`"domain"` as an array vs the
  current hex-string), so it won't parse. `for_network("mainnet")` returns a clean
  `VerifierError::VerificationKeyUnavailable` rather than panicking. **Fix:**
  regenerate that index in the current format upstream (we own `mina-rust`) —
  `mina-rust`'s `precalculate_block_verifier_index_and_srs` produces it.
- **mesa-mut** (hardfork network) — capture works and blocks **decode** with our
  types, but it's a different circuit: blocks do not verify against the devnet (or
  mainnet) VK, and no mesa-mut verifier index is embedded. Verifying it needs a
  caller-supplied VK (see below).

### Verifying arbitrary networks (mesa-mut, future forks)

Verification is gated on having the network's blockchain verifier index. mina-tree
embeds only devnet (current) and mainnet (stale). The general fix is a
`Verifier::with_index_json(&str)` that loads any VK — which needs `mina-rust`'s
`make_verifier_index` exposed as `pub` (it's the SRS/endo finalization a parsed JSON
index requires) plus the target network's index JSON. Both are within our control
since we own `mina-rust`.

## Dependency note

Pinned to `o1-labs/mina-rust @ ab69eaed` and `o1-labs/proof-systems @ 0.3.0`.
**Upstream (OpenMina) is unmaintained** — before this matters in production, fork
those repos (and the `o1-labs/rust-libp2p` fork) under o1-labs control and repoint
the workspace dependencies, so a deleted/rewritten upstream can't break the build.

## Status

- **Phase 0 — single-tip verification.** Verified a live devnet block end-to-end; a
  one-field tamper is rejected.
- **Phase 1 — consensus fork-choice + windowed monitor + verify-before-ingest.**
  - `Verifier::verify_tip` + `compare_tips`/`select_canonical` wrap Ouroboros
    Samasika (`mina_core::consensus`) over proof-verified tips.
  - `ChainMonitor` keeps a bounded window keyed by state hash, walks
    `previous_state_hash` links, and classifies each verified tip as
    **Extend / Reorg / Fork / Behind / Duplicate / Unlinked** — naming the
    divergence point. Validated on a **real same-height devnet fork** (height
    526718): GENESIS then FORK, common ancestor identified.
  - `mina-verify-monitor` is the **verify-before-ingest** consumer: every gossiped
    block's proof is verified (on a worker thread, off the gossip loop) before it
    enters the monitor; invalid blocks are rejected. Live run ingested consecutive
    devnet blocks 526735→526736 ("extends best"), 0 rejected.

Next: the mobile (UniFFI/WASM) binding; harden the monitor (peer discovery,
persistence); a real indexer integration.

## Run the live monitor

```sh
cargo run -p mina-verify-monitor          # verifies every devnet block before ingest
```
