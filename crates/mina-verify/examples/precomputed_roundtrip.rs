// Encode a captured devnet gossip block into precomputed-JSON form, then verify it —
// proving the precomputed decode produces a header whose proof checks (devnet VK).
// cargo run --example precomputed_roundtrip -p mina-verify -- <captured.gossipbin>
use base64::Engine;
use binprot::BinProtWrite;
use mina_verify::{block_from_gossip_payload, Verifier};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: <captured devnet gossip block>");
    let payload = std::fs::read(&path).expect("read block");
    let block = block_from_gossip_payload(&payload).expect("decode gossip block");
    let header = &block.header;

    let ps = serde_json::to_value(&header.protocol_state).expect("serialize protocol_state");
    let mut proof_bytes = Vec::new();
    header
        .protocol_state_proof
        .binprot_write(&mut proof_bytes)
        .expect("binprot proof");
    let proof_b64 = base64::engine::general_purpose::URL_SAFE.encode(&proof_bytes);

    let json = serde_json::json!({
        "data": { "protocol_state": ps, "protocol_state_proof": proof_b64 }
    })
    .to_string();

    let v = Verifier::for_network_offline("devnet").expect("devnet verifier");
    // the indexer's call: verify the proof AND get the proof-backed facts.
    let vb = v
        .verify_precomputed_and_extract(&json)
        .expect("proof did not verify");
    println!("verified precomputed block (devnet):");
    println!("  height              {}", vb.height);
    println!("  state_hash          {}", vb.state_hash);
    println!("  previous_state_hash {}", vb.previous_state_hash);
    println!(
        "  staged_ledger_hash  {}  <- indexer's replayed ledger root must match this",
        vb.staged_ledger_hash
    );
}
