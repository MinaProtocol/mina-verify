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

## Run

```sh
# native
MINA_NETWORK=devnet cargo run --release -p mina-verify-server

# docker (image builds all mina-verify bins; override the entrypoint)
docker run -e MINA_NETWORK=devnet -p 8090:8090 --entrypoint mina-verify-server <image>

curl -s localhost:8090/health
curl -s -X POST --data-binary @block.json localhost:8090/verify
```

Verification on a native release build is ~1–2 s per block, so an indexer can afford to
verify **every** block (not just the tip).

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
