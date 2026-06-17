//! Heavy, real-proof verification of a captured devnet block.
//!
//! A single SNARK verification is seconds in release and minutes in an unoptimized debug
//! build, so these are `#[ignore]`d — `cargo test` (and CI) skip them. Run explicitly:
//!
//! ```text
//! cargo test -p mina-verify --release -- --ignored
//! ```
//!
//! The fixture is `tests/fixtures/devnet-528700.json`, a real precomputed block pulled from
//! the public `mina_network_block_data` bucket. It is not strictly UTF-8 (the daemon emits
//! some byte-string fields raw), so it's decoded lossily — exactly as the server does.

use mina_verify::{Verifier, VerifierError};

const FIXTURE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/devnet-528700.json"
);

fn fixture_json() -> String {
    let bytes = std::fs::read(FIXTURE_PATH).expect("read fixture block");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
#[ignore = "heavy: runs a real SNARK verification — run with `--release -- --ignored`"]
fn verifies_real_devnet_block_and_extracts_facts() {
    let verifier = Verifier::for_network_offline("devnet").expect("embedded devnet VK");
    let vb = verifier
        .verify_precomputed_and_extract(&fixture_json())
        .expect("a genuine block's proof must verify");

    assert_eq!(vb.height, 528700);
    assert_eq!(
        vb.state_hash.to_string(),
        "3NKAteSXBXDELWVeTy3xLRe1WEPNzopApQfm21FK5PquH3H4xDks"
    );
    assert_eq!(
        vb.previous_state_hash.to_string(),
        "3NL48mjEeTVCaHdK8FrdmSALWrxiQcK1ZdBQgVJrQrLM74Pw6N73"
    );
    assert_eq!(
        vb.staged_ledger_hash.to_string(),
        "jweyWjnKbYcLxQUeMe4QjQxKziFARJicA5sCi4ZwCpWW6bxBCec"
    );
}

#[test]
#[ignore = "heavy: runs a real SNARK verification — run with `--release -- --ignored`"]
fn rejects_a_block_whose_proof_no_longer_matches() {
    let verifier = Verifier::for_network_offline("devnet").expect("embedded devnet VK");

    // Mutate the proven consensus state (block height). The block still decodes, but its
    // public input no longer matches the proof, so verification must reject it — and the
    // facts must NOT be extracted.
    let mut v: serde_json::Value = serde_json::from_str(&fixture_json()).unwrap();
    v["data"]["protocol_state"]["body"]["consensus_state"]["blockchain_length"] =
        serde_json::json!("999999");

    match verifier.verify_precomputed_and_extract(&v.to_string()) {
        Err(VerifierError::ProofInvalid) => {}
        other => panic!("expected ProofInvalid for a tampered block, got {other:?}"),
    }
}
