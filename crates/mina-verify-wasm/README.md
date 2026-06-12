# mina-verify-wasm

WebAssembly bindings for [`mina-verify`](../mina-verify): verify a Mina block's
Pickles/kimchi proof from JS/TS, with no native dependency. A verifying block attests
its entire chain history by Pickles recursion, so a tiny JS client can check chain
validity against an **untrusted** data source.

## API

```ts
import { verifyPrecomputed } from "mina-verify-wasm";

// precomputed-block JSON (GCS/archive form) -> proof-backed facts, or throws if the
// proof does not verify (=> do not ingest)
const facts = JSON.parse(verifyPrecomputed("devnet", precomputedJson));
// { height, stateHash, previousStateHash, stagedLedgerHash }
```

Only the **precomputed** (no-networking) path is exposed here; the live-node libp2p
path stays native (`mina-verify-monitor`).

## Build

See `build.sh`. The verifier stack is threaded wasm (mina-core → `wasm_thread`,
mina-tree → `rayon`), so it needs a **nightly** toolchain with `build-std` and
atomics/shared-memory — the recipe mirrors openmina's `crates/node/web` and lives in
`.cargo/config.toml`.

```sh
./build.sh nodejs     # or: web | bundler
```

## Status / performance

- ✅ Compiles to a ~4.7 MB release wasm; `verifyPrecomputed` runs in pure Node and
  returns facts identical to the native verifier (verified on a devnet block).
- ⚠️ **Single-threaded today (~70 s/verify).** Two known levers, both follow-ups:
  1. **Threaded runtime** — spin up the rayon worker pool (Workers + SharedArrayBuffer,
     openmina-web-node style). This is the big win; the wasm is already built threaded.
  2. **`num-bigint` opt-level** — the openmina `rebase-onstack` fork ICEs the compiler
     at `opt-level=3` on wasm, so the workspace pins it to `opt-level=1`
     (`[profile.release.package.num-bigint]`). That penalizes the hottest crate; a
     nightly where it builds at `-O3`, or an upstream fix, would recover a lot.

For a long-lived host (the MCP server), the SRS is cached after the first call, so the
cost is per-verify compute, not setup.
