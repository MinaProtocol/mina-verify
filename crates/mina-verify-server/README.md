# mina-verify-server

A long-lived HTTP **verify sidecar**: POST a precomputed block, get its proof-backed
facts. Built for a **trustless indexer** — verify each block's SNARK proof before
ingesting it, with no trusted daemon. A valid proof attests the entire chain to genesis
by recursion, so trust in whoever produced the block is not required.

Only the `mina-verify` library is linked (no networking — the precomputed path needs
none), so this stays a small, stateless, CPU-bound service. The expensive verifier setup
is paid once at startup; each request is just the proof check.

## Endpoints

```
GET  /health   -> { "status": "ok", "network": "devnet" }

POST /verify   (body = precomputed-block JSON)
  proof valid    -> 200 { "valid": true, "height", "state_hash",
                          "previous_state_hash", "staged_ledger_hash" }
  proof invalid  -> 200 { "valid": false, "error": "block proof did not verify" }
  undecodable    -> 400 { "valid": false, "error": "<detail>" }
```

The caller ingests iff `valid` is `true`, keyed by the returned (proof-backed) hashes.

> Real precomputed blocks are **not strictly UTF-8** — the OCaml daemon emits some
> byte-string fields (e.g. `sok_digest` in `staged_ledger_diff`) as mixed raw/escaped
> bytes. Those fields are ignored by verification, so the body is decoded lossily; this is
> expected and harmless.

## Config (env)

| var | meaning | default |
|-----|---------|---------|
| `BIND` | listen address | `0.0.0.0:8090` |
| `MINA_VK_JSON` | path to a blockchain verifier-index JSON (any network; **required for mesa / mesa-mut**). Takes precedence. | — |
| `MINA_NETWORK` | embedded-VK network when `MINA_VK_JSON` is unset (`devnet` / `mainnet`) | `devnet` |
| `VERIFY_THREADS` | worker threads (verification is CPU-bound) | available parallelism |

> When `MINA_VK_JSON` is set, `/health` reports `"network": "custom"` — the verifier
> index JSON carries the key, not the network name.

## Robustness

The body is **untrusted** input, so the service is hardened against a single bad request
taking down a worker:

- **Body cap** — at most 32 MiB is buffered per request (`MAX_BODY_BYTES`); a precomputed
  block is ~1 MB, so this is generous. An oversized body is truncated → the block fails to
  decode → a clean `400`, never an OOM.
- **Per-request panic isolation** — proof verification runs inside `catch_unwind`. A
  malformed-but-decodable block that trips an assertion deep in the verifier fails *that*
  request (`500 { "valid": false }`) instead of killing a worker thread.

## Run

```sh
# native
MINA_NETWORK=devnet cargo run --release -p mina-verify-server

# docker (image builds all mina-verify bins; override the entrypoint)
docker run -e MINA_NETWORK=devnet -p 8090:8090 --entrypoint mina-verify-server <image>

curl -s localhost:8090/health
curl -s -X POST --data-binary @block.json localhost:8090/verify
```

On a native release build, the warm steady-state is **~0.4 s per block** (~2.25
blocks/sec/core; see `cargo bench -p mina-verify`). The first verify is ~2.7 s (it primes
the globally-cached SRS) and building the verifier at startup is a further ~5 s — both
one-time costs the long-lived service pays once. So an indexer can afford to verify
**every** block, not just the tip.

## Trustless-indexer topology

```yaml
# docker-compose.yml
services:
  mina-verifier:
    image: mina-verify
    entrypoint: mina-verify-server
    environment:
      MINA_NETWORK: devnet          # or mount a VK and set MINA_VK_JSON for mesa
    # ports/healthcheck omitted for brevity

  mina-indexer:
    image: mina-indexer
    # reuse the existing exe-hook pattern: --verify-block-exe points at a shim that
    #   curls http://mina-verifier:8090/verify and exits non-zero unless {"valid":true}
    depends_on: [mina-verifier]
```

`verify-block.sh` shim (mirrors the indexer's `--fetch-new-blocks-exe` convention):

```sh
#!/usr/bin/env bash
# usage: verify-block.sh <precomputed-block.json>  — exit 0 iff the proof verifies
set -euo pipefail
curl -fsS -X POST --data-binary @"$1" "${VERIFIER_URL:-http://mina-verifier:8090}/verify" \
  | grep -q '"valid":true'
```

The sidecar's Rust toolchain (1.94.1 + mina-verify's patched lock) stays bottled up in its
own container — it never touches the indexer's build.

## Tests

```sh
# fast: router + endpoint behaviour (no SNARK proof runs). The one slow step is the
# debug-mode VK parse at startup, so it's built once and shared.
cargo test -p mina-verify-server

# heavy: real end-to-end proof verification of a captured block (valid + tampered).
# Seconds in release, so run release:
cargo test -p mina-verify-server --release -- --ignored
```

- `tests/dispatch.rs` — pure router unit tests (`dispatch`): health, 404, and the
  `400 valid:false` error mapping, against a shared embedded-VK verifier.
- `tests/acceptance.rs` — spawns the real binary on an ephemeral port and drives it over a
  socket (dependency-free HTTP/1.1 client). The fast test covers health / bad-request /
  not-found; the `#[ignore]`d tests POST a real block (→ `valid:true` with the right
  hashes) and a tampered one (→ `valid:false`).
