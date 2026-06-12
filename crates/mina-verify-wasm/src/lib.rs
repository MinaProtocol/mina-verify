//! WebAssembly bindings for `mina-verify`.
//!
//! Exposes the **precomputed-block** verify path to JS/TS: given a network name and a
//! precomputed-block JSON, verify its Pickles/kimchi proof and return the proof-backed
//! facts (height, state hash, parent, staged-ledger root) as a JSON string. The proof
//! verifies the entire chain history up to that block by Pickles recursion — no trust
//! in whoever supplied the JSON.
//!
//! Only the precomputed (no-networking) path is exposed here; the live-node (libp2p)
//! path stays native (`mina-verify-monitor`).

use wasm_bindgen::prelude::*;

/// Verify a precomputed-block JSON against `network`'s embedded verification key.
///
/// `network` is "devnet" or "mainnet" (uses the embedded VK without touching any
/// process-global config). Returns a JSON string
/// `{ "height", "stateHash", "previousStateHash", "stagedLedgerHash" }` on success, or
/// a JS error (string) if the JSON is malformed or the proof does not verify — in which
/// case the block must NOT be ingested.
#[wasm_bindgen(js_name = verifyPrecomputed)]
pub fn verify_precomputed(network: &str, precomputed_json: &str) -> Result<String, JsValue> {
    console_error_panic_hook::set_once();
    let verifier = mina_verify::Verifier::for_network_offline(network)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let vb = verifier
        .verify_precomputed_and_extract(precomputed_json)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let out = serde_json::json!({
        "height": vb.height,
        "stateHash": vb.state_hash.to_string(),
        "previousStateHash": vb.previous_state_hash.to_string(),
        "stagedLedgerHash": vb.staged_ledger_hash.to_string(),
    });
    Ok(out.to_string())
}
