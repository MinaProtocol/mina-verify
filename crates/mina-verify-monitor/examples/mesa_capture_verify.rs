// Capture a live mesa-mut block off gossip (mesa types, no re-serialization) and
// verify it with the mesa VK — definitive end-to-end test on the mesa kimchi stack.
//   cargo run --example mesa_capture_verify -p mina-verify-monitor -- <mesa_vk.json>
use mina_verify::{block_from_gossip_payload, Verifier};
use mina_verify_capture::subscribe_blocks;
use std::ops::ControlFlow;
use std::time::Duration;

const CHAIN_ID: &str = "8b8ccbf273ef48aa0193ed634e69540657f0fc4292c9919a54b76a21b104abb2";
const PEERS: &[&str] = &[
    "/ip4/65.109.48.175/tcp/8302/p2p/12D3KooWKE8NSGK4VDtNas1MiGZ1MkerDpLNkwbW19mbieVSHh5x",
    "/ip4/212.83.33.149/tcp/60201/p2p/12D3KooWCDEHfWbspmBK3eoujvdiGCbJimwtxRPorUbNpmbL13Dn",
    "/ip4/15.235.55.176/tcp/8302/p2p/12D3KooWFQ7GmguCqSQUQZZbwDZiuBpdaNhyMM7SYz5yS9obz11o",
    "/ip4/65.109.37.38/tcp/8302/p2p/12D3KooWKsDeitYKn7EkBCqSU7hrfrW4DyQopQ5WWiV3E6ie3H7b",
    "/ip4/178.105.168.66/tcp/8302/p2p/12D3KooWBoHDNC34A4oM5odpeBzq3Jrq2Zror5c5krYdSKPUgA36",
    "/ip4/167.233.27.225/tcp/18415/p2p/12D3KooWCpxLfDvbKxWaHxQP1rDXxxA7YsVXU47KYmX4YcKzHTLD",
    "/ip4/157.180.1.110/tcp/1940/p2p/12D3KooWLfer8ojmrC1a2km5rdZSZsz3aDwLUby6Puz3iR2qQgqA",
    "/ip4/65.109.4.219/tcp/8302/p2p/12D3KooWJS5zBTbBZKnrQJ46e9irS84du426CZZgEVjs32Bp5LoX",
    "/ip4/65.108.0.140/tcp/8302/p2p/12D3KooWKeRQ9ePgDA8DcNrRzemLsbbpA2TgaEkrNcTqKafCrPP3",
    "/ip4/190.102.106.6/tcp/8302/p2p/12D3KooWEJnxVrivuCNz4BjKtTU6zsWaFojSxgGSCjbBQ7sq1Xry",
    "/ip4/15.235.230.161/tcp/8302/p2p/12D3KooWAm9evB8m5Q9djPzV3xRPq1RNo9JodNwaGz5DMbfuKEr1",
    "/ip4/82.67.133.101/tcp/14315/p2p/12D3KooWPNzQfrofi1rQzgsExUujbJ3RDj9m794duUbzZNpfKKE1",
    "/ip4/54.36.165.140/tcp/8302/p2p/12D3KooWNTtJmyVaFMyBzAXyNsPu1sCrBAaeZxxacxKou5r2X92b",
    "/ip4/71.193.174.199/tcp/8302/p2p/12D3KooWFZSRE4Kx2kBvgoCvetjZE7gTLgqFahsNzcH5Swz4W1dq",
    "/ip4/139.84.156.198/tcp/8302/p2p/12D3KooWLbDdXXcsBBJXVFQaBcQWgDyxF4r1EuqqGfVaYf71QNpZ",
    "/ip4/157.180.1.108/tcp/13085/p2p/12D3KooWEaF1ED3t2FyLhPirybL83puLBSUfFTzeeTq16pCZyAEi",
    "/ip4/128.116.219.252/tcp/8303/p2p/12D3KooWPEoKvoDk4iA7yD9VsbRH1LkuPqUgHSnhjqFe7gpkKcAc",
    "/ip4/178.104.123.4/tcp/8032/p2p/12D3KooWFj8VfrHCfTQ5dHNEBUVjM76m7M35wPDVyMaJ52aJDvxm",
];

#[tokio::main]
async fn main() {
    env_logger::init();
    let vk_path = std::env::args().nth(1).expect("usage: <mesa_vk.json>");
    let json = std::fs::read_to_string(&vk_path).expect("read vk");
    let verifier = Verifier::with_index_json(&json).expect("build verifier from mesa vk");
    println!(
        "subscribing to mesa-mut gossip ({} peers, up to 300s)...",
        PEERS.len()
    );
    let mut done = false;
    subscribe_blocks(
        CHAIN_ID,
        PEERS,
        Some(Duration::from_secs(300)),
        |payload| {
            let block = match block_from_gossip_payload(payload) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("decode err: {e}");
                    return ControlFlow::Continue(());
                }
            };
            let h = block
                .header
                .protocol_state
                .body
                .consensus_state
                .blockchain_length
                .as_u32();
            let sh = block
                .header
                .try_hash()
                .map(|x| x.to_string())
                .unwrap_or_else(|e| format!("{e:?}"));
            let ok = verifier.verify_block(&block);
            println!("\nmesa-mut block height {h} computed_state_hash={sh} -> verify_block = {ok}");
            done = true;
            ControlFlow::Break(())
        },
        |peers| {
            eprintln!("  connected peers: {peers}");
            ControlFlow::Continue(())
        },
    )
    .await;
    if !done {
        eprintln!("no block within deadline");
        std::process::exit(1);
    }
}
