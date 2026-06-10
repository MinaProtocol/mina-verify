//! Verify a Mina block proof from a captured consensus-gossip payload.
//!
//! Usage: `mina-verify <gossip-payload-file>`
//! (produce a payload file with the `mina-verify-capture` binary).

use mina_verify::{block_from_gossip_payload, Verifier};

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: mina-verify <gossip-payload-file>");
            std::process::exit(2);
        }
    };

    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {path}: {e}");
        std::process::exit(2);
    });

    let block = block_from_gossip_payload(&bytes).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(2);
    });

    let height = &block
        .header
        .protocol_state
        .body
        .consensus_state
        .blockchain_length
        .0
         .0;

    let verifier = Verifier::devnet();
    let ok = verifier.verify_block(&block);

    println!("devnet block height {height}: verify_block = {ok}");
    std::process::exit(if ok { 0 } else { 1 });
}
