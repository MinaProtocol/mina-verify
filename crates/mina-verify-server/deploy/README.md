# Trustless-indexer deployment

Glue for gating an indexer's ingestion on the verify sidecar.

- **`docker-compose.yml`** — runs `mina-verifier` (the sidecar) with a `/health`
  healthcheck, plus a commented `mina-indexer` stub showing the wiring.
- **`verify-block.sh`** — the `--verify-block-exe` shim: `curl`s the sidecar's
  `/verify` and exits `0` iff `{"valid":true}`. The indexer calls it at canonical
  promotion; non-zero ⇒ reject the block.

## Try the gating contract (devnet, no indexer needed)

```sh
# 1. start the sidecar (native or container)
MINA_NETWORK=devnet cargo run --release -p mina-verify-server     # :8090
#   or: docker compose -f docker-compose.yml up mina-verifier

# 2. point the shim at it and gate a real block vs a tampered one
VERIFIER_URL=http://127.0.0.1:8090 ./verify-block.sh real-block.json   ; echo "exit=$?"  # 0  → ingest
VERIFIER_URL=http://127.0.0.1:8090 ./verify-block.sh tampered.json     ; echo "exit=$?"  # !0 → reject
```

## Wiring into the indexer

The indexer reuses its existing exe-hook pattern (like `--fetch-new-blocks-exe`):
pass `--verify-block-exe /app/verify-block.sh` and set `VERIFIER_URL` to the sidecar.
At canonical extension the indexer shells out per block; `valid:true` ⇒ persist,
otherwise reject and log. Gate only at canonical promotion (not losing forks), and
on a mainnet-scale backfill verify the tip + checkpoints rather than every historical
block (recursion already backs the ancestry). At mesa scale, verify-all is fine
(~1–2 s/block).

## Networks

devnet/mainnet use the embedded VK. For **mesa / mesa-mut**, mount a verifier-index
JSON into the sidecar and set `MINA_VK_JSON` (see the compose comments) — same shim,
same demo, no code change.
