// Verify a block using a VK ingested from the OCaml print_blockchain_snark_vk
// (Pickles.Verification_key) JSON — used to validate the mesa VK end-to-end.
//   cargo run --example pickles_vk_verify -p mina-verify -- <pickles_vk.json> <block.binprot>
use mina_verify::{block_from_binprot, Verifier};

fn main() {
    let vk_path = std::env::args().nth(1).expect("usage: <pickles_vk.json> <block.binprot>");
    let blk_path = std::env::args().nth(2).expect("usage: <pickles_vk.json> <block.binprot>");
    let json = std::fs::read_to_string(&vk_path).expect("read vk json");
    let verifier = Verifier::with_index_json(&json).expect("build verifier from pickles vk");
    let ok = if blk_path.ends_with(".json") {
        let block_json = std::fs::read_to_string(&blk_path).expect("read precomputed block");
        verifier.verify_precomputed_block(&block_json).expect("decode precomputed block")
    } else {
        let bytes = std::fs::read(&blk_path).expect("read block");
        let block = block_from_binprot(&bytes).expect("decode block");
        let h = block.header.protocol_state.body.consensus_state.blockchain_length.as_u32();
        let sh = block.header.try_hash().map(|x| x.to_string()).unwrap_or_else(|e| format!("{e:?}"));
        let prev = &block.header.protocol_state.previous_state_hash;
        eprintln!("block height={h} computed_state_hash={sh} previous_state_hash={prev}");
        verifier.verify_block(&block)
    };
    println!("verify_block = {ok}");
    std::process::exit(if ok { 0 } else { 2 });
}
