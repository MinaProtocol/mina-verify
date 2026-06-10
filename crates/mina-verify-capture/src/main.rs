// Capture a current devnet block by subscribing to Mina's consensus gossip topic.
//
// Mina gossip payloads are: [u64-LE length][GossipNetMessageV2 binprot]. So byte 8
// is the enum tag: 0 = NewState (a block), 1 = SnarkPoolDiff, 2 = TransactionPoolDiff.
// We save the whole payload for any large message (blocks are ~14 KB) and let the
// verifier-side decoder strip the 8-byte prefix.

use std::{fs, io::Write, path::PathBuf, time::Duration};

mod transport;

use libp2p::{futures::StreamExt, gossipsub, swarm::SwarmEvent, Multiaddr};
use transport::ed25519::{Keypair as EdKeypair, SecretKey};

const CHAIN_ID: &str = "29936104443aaf264a7f0192ac64b1c7173198c1ed404c1bcff5e562e05eb7f6";
const PEERS: &[&str] = &[
    "/dns4/seed-1.devnet.gcp.o1test.net/tcp/10003/p2p/12D3KooWAdgYL6hv18M3iDBdaK1dRygPivSfAfBNDzie6YqydVbs",
    "/dns4/seed-2.devnet.gcp.o1test.net/tcp/10003/p2p/12D3KooWLjs54xHzVmMmGYb7W5RVibqbwD1co7M2ZMfPgPm7iAag",
    "/dns4/seed-3.devnet.gcp.o1test.net/tcp/10003/p2p/12D3KooWEiGVAFC7curXWXiGZyMWnZK9h8BKr88U8D5PKV3dXciv",
];

#[tokio::main]
async fn main() {
    env_logger::init();

    let out = PathBuf::from("captured");
    fs::create_dir_all(&out).unwrap();

    let peers: Vec<Multiaddr> = PEERS.iter().map(|s| s.parse().unwrap()).collect();

    let local_key: libp2p::identity::Keypair = EdKeypair::from(SecretKey::generate()).into();
    eprintln!("local peer id: {}", local_key.public().to_peer_id());

    let behaviour = {
        let cfg = gossipsub::ConfigBuilder::default()
            .max_transmit_size(1024 * 1024 * 32)
            .build()
            .expect("valid gossipsub config");
        let b: gossipsub::Behaviour =
            gossipsub::Behaviour::new(gossipsub::MessageAuthenticity::Signed(local_key.clone()), cfg)
                .expect("gossipsub behaviour");
        b
    };

    // pnet PSK = Blake2b256("/coda/0.0.1/" || chain_id_hex). mina_transport::swarm
    // hashes exactly the bytes we pass, so we pass the already-prefixed string.
    let pnet_input = format!("/coda/0.0.1/{CHAIN_ID}");
    let listen: Vec<Multiaddr> = vec![];
    let mut swarm = transport::swarm(
        local_key,
        pnet_input.as_bytes(),
        listen,
        peers.iter().cloned(),
        behaviour,
    );

    let topic = gossipsub::IdentTopic::new("coda/consensus-messages/0.0.1");
    swarm.behaviour_mut().subscribe(&topic).unwrap();
    for peer in &peers {
        for proto in peer.iter() {
            if let libp2p::multiaddr::Protocol::P2p(peer_id) = proto {
                swarm.behaviour_mut().add_explicit_peer(&peer_id);
            }
        }
    }

    let mut saved = 0usize;
    let secs: u64 = std::env::var("CAPTURE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(600);
    let deadline = tokio::time::sleep(Duration::from_secs(secs));
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => { eprintln!("timeout reached; saved {saved} message(s)"); break; }
            ev = swarm.next() => match ev {
                Some(SwarmEvent::Behaviour(gossipsub::Event::Message { message, .. })) => {
                    let d = &message.data;
                    let tag = d.get(8).copied();
                    let kind = match tag { Some(0) => "NewState(BLOCK)", Some(1) => "SnarkPoolDiff", Some(2) => "TxPoolDiff", _ => "?" };
                    eprintln!("gossip msg: len={} tag@8={:?} [{kind}]", d.len(), tag);
                    // Only a NewState (tag 0) is a block. Save it; ignore pool diffs.
                    if tag == Some(0) {
                        let p = out.join(format!("block-{saved}.gossipbin"));
                        fs::File::create(&p).unwrap().write_all(d).unwrap();
                        eprintln!("  -> SAVED BLOCK {} ({} bytes)", p.display(), d.len());
                        saved += 1;
                        if saved >= 1 { eprintln!("captured a block; exiting"); break; }
                    }
                }
                Some(SwarmEvent::ConnectionEstablished { peer_id, .. }) => eprintln!("connected: {peer_id}"),
                Some(SwarmEvent::Behaviour(gossipsub::Event::Subscribed { peer_id, topic })) => {
                    eprintln!("peer subscribed: {peer_id} {topic}")
                }
                Some(SwarmEvent::OutgoingConnectionError { peer_id, error, .. }) => {
                    eprintln!("DIAL ERROR to {peer_id:?}: {error}")
                }
                Some(SwarmEvent::Dialing { peer_id, .. }) => eprintln!("dialing {peer_id:?}"),
                Some(ev) => eprintln!("event: {ev:?}"),
                None => break,
            }
        }
    }
}
