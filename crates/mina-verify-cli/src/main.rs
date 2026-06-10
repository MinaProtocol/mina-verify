//! Verify Mina block proofs from captured consensus-gossip payloads.
//!
//! Usage:
//!   mina-verify <payload>                 verify one block's proof
//!   mina-verify <payload> <payload> ...   verify each, feed a chain monitor, and
//!                                          classify each tip (extend / fork / reorg)
//!
//! (produce payload files with the `mina-verify-capture` binary)

use std::process::exit;

use mina_verify::{block_from_gossip_payload, ChainMonitor, Ingest, VerifiedTip, Verifier};

fn load_and_verify(verifier: &Verifier, path: &str) -> VerifiedTip {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {path}: {e}");
        exit(2);
    });
    let block = block_from_gossip_payload(&bytes).unwrap_or_else(|e| {
        eprintln!("error: {path}: {e}");
        exit(2);
    });
    match verifier.verify_tip(block) {
        Ok(Some(tip)) => tip,
        Ok(None) => {
            println!("{path}: verify_block = false (proof rejected)");
            exit(1);
        }
        Err(e) => {
            eprintln!("error: {path}: malformed block: {e:?}");
            exit(2);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: mina-verify <payload> [<payload> ...]");
        exit(2);
    }

    let network = std::env::var("MINA_NETWORK").unwrap_or_else(|_| "devnet".into());
    let verifier = Verifier::for_network(&network).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        exit(2);
    });

    if args.len() == 1 {
        let tip = load_and_verify(&verifier, &args[0]);
        println!("{}: height {} — verify_block = true", args[0], tip.height());
        return;
    }

    // Chain-monitor mode: verify each tip, ingest, classify.
    let mut monitor = ChainMonitor::new(512);
    for path in &args {
        let tip = load_and_verify(&verifier, path);
        let height = tip.height();
        let outcome = monitor.ingest(&tip);
        let note = match &outcome {
            Ingest::Genesis => "first tip — adopted as best".to_string(),
            Ingest::Extend { .. } => "extends best chain".to_string(),
            Ingest::Duplicate => "duplicate (already seen)".to_string(),
            Ingest::Behind { .. } => "behind best (orphan/older, same chain)".to_string(),
            Ingest::Reorg { common_ancestor, depth, .. } => format!(
                "REORG — new canonical tip; diverged at {} (rolled back {} block(s))",
                common_ancestor.as_deref().unwrap_or("<unknown>"),
                depth.map(|d| d.to_string()).unwrap_or_else(|| "?".into())
            ),
            Ingest::Fork { common_ancestor } => format!(
                "FORK — competing branch (best unchanged); diverged at {}",
                common_ancestor.as_deref().unwrap_or("<unknown>")
            ),
            Ingest::Unlinked => "UNLINKED — common ancestor outside window".to_string(),
        };
        println!("verified height {height:<7} [{}] {note}", variant(&outcome));
    }

    if let (Some(h), Some(hash)) = (monitor.best_height(), monitor.best()) {
        println!("\ncanonical best: height {h}, {hash}");
    }
}

fn variant(i: &Ingest) -> &'static str {
    match i {
        Ingest::Genesis => "GENESIS",
        Ingest::Extend { .. } => "EXTEND",
        Ingest::Duplicate => "DUP",
        Ingest::Behind { .. } => "BEHIND",
        Ingest::Reorg { .. } => "REORG",
        Ingest::Fork { .. } => "FORK",
        Ingest::Unlinked => "UNLINKED",
    }
}
