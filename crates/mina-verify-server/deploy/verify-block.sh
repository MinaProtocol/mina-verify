#!/usr/bin/env bash
# Indexer --verify-block-exe shim: exit 0 iff the block's SNARK proof verifies.
#
# At canonical promotion the indexer calls this with the precomputed-block file; a
# non-zero exit means "reject — do not ingest". Mirrors the --fetch-new-blocks-exe
# convention (a thin curl wrapper, like mesa-pull), so the warm verifier lives in the
# sidecar and the indexer stays toolchain-clean.
#
#   usage: verify-block.sh <precomputed-block.json>
#   env:   VERIFIER_URL   (default http://mina-verifier:8090)
#          VERIFY_TIMEOUT seconds (default 120)
set -euo pipefail

block="${1:?usage: verify-block.sh <precomputed-block.json>}"
url="${VERIFIER_URL:-http://mina-verifier:8090}/verify"

# curl -f => non-zero on HTTP errors; set -e => abort the script on that.
resp="$(curl -fsS --max-time "${VERIFY_TIMEOUT:-120}" \
  -H 'Content-Type: application/json' --data-binary @"$block" "$url")"

echo "verify-block: $resp" >&2   # surface the verdict in indexer logs

# Final command's status is the script's exit status: 0 iff the proof verified.
printf '%s' "$resp" | grep -q '"valid":true'
