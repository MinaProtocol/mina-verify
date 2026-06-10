//! Verify Mina block proofs from captured consensus-gossip payloads.
//!
//! Usage:
//!   mina-verify <payload>             verify one block's proof
//!   mina-verify <payload> <payload2>  verify both, then apply Samasika fork-choice
//!
//! (produce payload files with the `mina-verify-capture` binary)

use std::process::exit;

use mina_verify::{block_from_gossip_payload, compare_tips, TipComparison, VerifiedTip, Verifier};

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
        Ok(Some(tip)) => {
            println!("{path}: height {} — verify_block = true", tip.height());
            tip
        }
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
    let verifier = Verifier::devnet();

    match args.as_slice() {
        [a] => {
            load_and_verify(&verifier, a);
        }
        [a, b] => {
            let ta = load_and_verify(&verifier, a);
            let tb = load_and_verify(&verifier, b);
            match compare_tips(&ta, &tb) {
                TipComparison::Same => {
                    println!("fork-choice: tips AGREE (identical state hash)");
                }
                TipComparison::Diverged {
                    canonical_is_b,
                    range,
                } => {
                    let (winner, w_height) = if canonical_is_b {
                        (b, tb.height())
                    } else {
                        (a, ta.height())
                    };
                    println!(
                        "fork-choice: DIVERGENCE ({range:?}-range) — canonical = {winner} (height {w_height})"
                    );
                }
            }
        }
        _ => {
            eprintln!("usage: mina-verify <payload> [<payload2>]");
            exit(2);
        }
    }
}
