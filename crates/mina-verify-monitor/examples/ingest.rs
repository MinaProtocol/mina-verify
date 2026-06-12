//! Verify one block from either source and print its proof-backed facts.
//!
//!   # a precomputed block (GCS / archive JSON) — the canonical starting point
//!   cargo run --example ingest -p mina-verify-monitor -- precomputed block.json
//!
//!   # a live node's best tip, fetched over libp2p RPC (the seed path)
//!   cargo run --example ingest -p mina-verify-monitor -- node devnet
//!
//! Verifier VK: MINA_VK_JSON=<file> for any network (mesa-mut / regenerated mainnet),
//! else the embedded VK for the named network (precomputed defaults to MINA_NETWORK
//! or "devnet"; node uses the network argument).

use std::process::exit;

use mina_verify::Verifier;
use mina_verify_monitor::{verify_from, BlockSource};

#[tokio::main]
async fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();

    let (source, network) = match args.as_slice() {
        [kind, path] if kind == "precomputed" => {
            let json = std::fs::read_to_string(path).unwrap_or_else(|e| {
                eprintln!("error: cannot read {path}: {e}");
                exit(2);
            });
            let network = std::env::var("MINA_NETWORK").unwrap_or_else(|_| "devnet".into());
            (BlockSource::Precomputed(json), network)
        }
        [kind, network] if kind == "node" => (BlockSource::node(network.clone()), network.clone()),
        _ => {
            eprintln!("usage:\n  ingest precomputed <file.json>\n  ingest node <devnet|mainnet|mesa-mut>");
            exit(2);
        }
    };

    // VK selection mirrors the other examples: explicit JSON wins, else embedded VK.
    let verifier = match std::env::var("MINA_VK_JSON") {
        Ok(p) => {
            let json = std::fs::read_to_string(&p).unwrap_or_else(|e| {
                eprintln!("error: cannot read MINA_VK_JSON {p}: {e}");
                exit(2);
            });
            Verifier::with_index_json(&json)
        }
        Err(_) => Verifier::for_network(&network),
    }
    .unwrap_or_else(|e| {
        eprintln!("error: {e}");
        exit(2);
    });

    match verify_from(&verifier, source).await {
        Ok(vb) => {
            println!("verified block ({network}):");
            println!("  height              {}", vb.height);
            println!("  state_hash          {}", vb.state_hash);
            println!("  previous_state_hash {}", vb.previous_state_hash);
            println!(
                "  staged_ledger_hash  {}  <- an indexer's replayed ledger root must match this",
                vb.staged_ledger_hash
            );
        }
        Err(e) => {
            eprintln!("NOT verified — do not ingest: {e}");
            exit(1);
        }
    }
}
