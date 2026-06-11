//! Capture live devnet blocks off the consensus-gossip network to disk.
//!
//! Env: CAPTURE_BLOCKS (how many, default 1), CAPTURE_SECS (timeout, default 600).
//! Writes `captured/block-N.gossipbin` (raw gossip payloads, ready for
//! `mina-verify <file>`).

use std::{fs, io::Write, ops::ControlFlow, path::PathBuf, time::Duration};

use mina_verify_capture::{network_seeds, subscribe_blocks};

#[tokio::main]
async fn main() {
    env_logger::init();

    let out = PathBuf::from("captured");
    fs::create_dir_all(&out).unwrap();

    let network = std::env::var("MINA_NETWORK").unwrap_or_else(|_| "devnet".into());
    let (chain_id, peers) = network_seeds(&network)
        .unwrap_or_else(|| panic!("unknown MINA_NETWORK {network:?} (devnet|mainnet)"));
    eprintln!("network: {network}");

    let want: usize = env("CAPTURE_BLOCKS", 1);
    let secs: u64 = env("CAPTURE_SECS", 600);

    let mut saved = 0usize;
    subscribe_blocks(
        chain_id,
        peers,
        Some(Duration::from_secs(secs)),
        |payload| {
            let p = out.join(format!("block-{saved}.gossipbin"));
            fs::File::create(&p).unwrap().write_all(payload).unwrap();
            eprintln!("SAVED BLOCK {} ({} bytes)", p.display(), payload.len());
            saved += 1;
            if saved >= want {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        },
        |_| ControlFlow::Continue(()),
    )
    .await;

    eprintln!("captured {saved} block(s)");
}

fn env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
