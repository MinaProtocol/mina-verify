// cargo run --example precomputed_decode -p mina-verify -- <precomputed-block.json>
use mina_verify::header_from_precomputed;
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: precomputed_decode <file.json>");
    let json = std::fs::read_to_string(&path).expect("read file");
    let header = header_from_precomputed(&json).expect("decode precomputed block");
    let h = header
        .protocol_state
        .body
        .consensus_state
        .blockchain_length
        .as_u32();
    let prev = &header.protocol_state.previous_state_hash;
    println!("decoded OK — height {h}, prev_state_hash {prev}");
}
