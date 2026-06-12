//! Convert a captured gossip block (`*.gossipbin` from `mina-verify-capture`) into the
//! precomputed-block JSON form, printed to stdout — handy for producing a
//! `BlockSource::Precomputed` input from a block you captured off the network.
//!
//!   cargo run --example to_precomputed -p mina-verify-monitor -- captured/block-0.gossipbin > block.json
use base64::Engine;
use binprot::BinProtWrite;
use mina_verify::block_from_gossip_payload;

fn main() {
    let path = std::env::args().nth(1).expect("usage: to_precomputed <captured.gossipbin>");
    let payload = std::fs::read(&path).expect("read captured block");
    let block = block_from_gossip_payload(&payload).expect("decode gossip block");
    let header = &block.header;

    let protocol_state = serde_json::to_value(&header.protocol_state).expect("serialize protocol_state");
    let mut proof_bytes = Vec::new();
    header.protocol_state_proof.binprot_write(&mut proof_bytes).expect("binprot proof");
    let protocol_state_proof = base64::engine::general_purpose::URL_SAFE.encode(&proof_bytes);

    let json = serde_json::json!({ "data": { "protocol_state": protocol_state, "protocol_state_proof": protocol_state_proof } });
    println!("{}", serde_json::to_string(&json).expect("serialize precomputed json"));
}
