// Verify a precomputed devnet block using the EMBEDDED devnet VK path
// (for_network_offline -> BlockVerifier embedded VK), bypassing with_index_json.
use mina_verify::Verifier;

fn main() {
    let blk_path = std::env::args().nth(1).expect("usage: <block.json>");
    let verifier = Verifier::for_network_offline("devnet").expect("build devnet verifier (embedded VK)");
    let block_json = std::fs::read_to_string(&blk_path).expect("read precomputed block");
    let ok = verifier.verify_precomputed_block(&block_json).expect("decode precomputed block");
    println!("embedded-VK verify_block = {ok}");
    std::process::exit(if ok { 0 } else { 2 });
}
