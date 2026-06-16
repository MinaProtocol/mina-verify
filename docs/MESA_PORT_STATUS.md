# Mesa verification in mina-verify (Rust) — status & handoff

Goal: make `mina-verify` (Rust, for mobile/wasm) verify **mesa-mut** block proofs.
Status: **not yet verifying**, but de-risked and narrowed to one remaining area.

## TL;DR
- The "different proof system" is real: mesa uses `o1-labs/proof-systems` **`native/mesa`
  (`ab84160`, "0.3.0-518-…")**; openmina/`mina-verify` pinned `0.3.0`.
- Bumping kimchi **is necessary but not sufficient**. The protocol layer is already
  mesa-compatible, but the **Pickles wrap-verification internals** in `mina-tree` are
  still pre-mesa.

## What works now (done)
1. **Kimchi bump.** `mina-rust` (`dkijania`, branch `mesa/proof-systems-bump`) repointed
   to the mesa `proof-systems` (editable local path = the OCaml submodule at `ab84160`).
2. **`mina-tree` compiles** against mesa kimchi after 3 fix classes:
   - `ScalarChallenge<F>(F)` → `(pub F)` in `proof-systems/poseidon/src/sponge.rs`.
   - `SRS.lagrange_bases` made `pub` in `proof-systems/poly-commitment/src/ipa.rs`;
     `caching.rs` adapted to the new `HashMapCache` API (`From<…> for HashMap`, `set_once`).
   - `permutation_vanishing_polynomial_m`/`w` are now `std::sync::OnceLock` (not
     `once_cell::OnceCell`) — `once_lock()` helper in `caching.rs` + `verifiers.rs`.
3. **Pickles-VK ingester** (`mina-verify`, branch `verify/mesa-support`):
   `verifier_index::verifier_index_from_pickles_json` + `verifier_index_auto`; accepted by
   `Verifier::with_index_json`/`MINA_VK_JSON`. **Validated correct**: every field of the
   ingested VK (domain, shift, public, prev_challenges, max_poly_size, all commitments)
   matches the known-good reference; commitments are on-curve; `finalize` matches mesa's
   canonical `make_verifier_index` byte-for-byte.
4. **mina-verify builds** against the mesa fork (deps repointed to local paths).
5. **OCaml VK generator** (`mina/src/lib/blockchain_snark/tests/print_blockchain_snark_vk`):
   added runtime-config fork support; `mesa_runtime_config.json` (fork @ 297422) produces
   the mesa VK (`/tmp/mesa_vk_pickles.json`).

## The remaining blocker (task: port Pickles internals)
A real mesa block (fresh capture, height 301406) fails verification, localized precisely:
- `accumulator_check` = **true** (proof + SRS + kimchi consistent).
- mina-tree's **computed state hash matches the daemon exactly** (301406 →
  `3NKV1EWCLp8uRpSXX16TFrTXZd4aRU5UF21ywnH8cSvaXmE7YzH4`) ⇒ protocol decode, poseidon, and
  state hashing are mesa-correct; the public-input `app_state` is right.
- The VK is proven correct (above).
- **But `verify_with(vk, proof, public_inputs)` returns `OpenProof`.**

By elimination, the gap is in `mina-tree`'s pre-mesa **Pickles wrap-verification internals**
(`crates/ledger/src/proofs/verification.rs::verify_impl` and friends):
`compute_deferred_values`, `get_message_for_next_wrap_proof`, `get_prepared_statement`,
`to_public_input`, `make_padded_proof_from_p2p`, `run_checks` — and possibly the proof
parsing in `mina-p2p-messages`. These must be ported to match mesa's Pickles changes
(see the OCaml mesa commits: "remove zkapp_spec from cs digests", "Don't check sok message
when verify tx snark", the `chunking`/plonk_types changes).

## Reproduce
```
# mina-verify @ verify/mesa-support (deps point at dkijania/mina-rust @ mesa/proof-systems-bump
# and the OCaml proof-systems submodule @ ab84160)
cargo run --example pickles_vk_verify -p mina-verify -- /tmp/mesa_vk_pickles.json /tmp/mesa_block.bin
# debug eprintln in mina-rust verification.rs prints: accum_check=true verify_impl=Ok(false), verify error=OpenProof
```
Fresh-capture variant (mesa types, no re-serialization): `mesa_capture_verify` example.

## Notes
- All dep repoints use **absolute local paths** (spike wiring) — do not commit as-is.
- proof-systems edits are 2 one-line `pub` changes (submodule, detached HEAD).
- A temporary debug `eprintln!` is in `mina-rust .../verification.rs::verify_block`.
