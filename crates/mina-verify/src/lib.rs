//! Trustless verification of Mina blocks.
//!
//! Given block bytes obtained from an *untrusted* source (a node, an indexer, a
//! captured gossip message), verify the block's Pickles/kimchi blockchain SNARK
//! proof against the network verification key. If the proof verifies, the entire
//! chain history up to that block is valid by Pickles recursion — no trust in the
//! source is required.
//!
//! ```no_run
//! let verifier = mina_verify::Verifier::devnet();
//! let block = mina_verify::block_from_gossip_payload(&payload)?;
//! assert!(verifier.verify_block(&block));
//! # Ok::<(), mina_verify::DecodeError>(())
//! ```
//!
//! The verification core (`verify_block`, the verification key, the SRS) is
//! provided by OpenMina's `mina-tree`; this crate is a thin, ergonomic wrapper.

pub mod fork_choice;
pub mod monitor;

pub use fork_choice::{
    compare_tips, is_short_range_fork, prefers_candidate, select_canonical, ForkRange,
    TipComparison, VerifiedTip,
};
pub use monitor::{ChainMonitor, Ingest};

use std::sync::Once;

use binprot::BinProtRead;
use mina_curves::pasta::Fp;
use mina_p2p_messages::gossip::GossipNetMessageV2;
use mina_tree::proofs::verification::verify_block as verify_block_proof;
use mina_tree::proofs::verifiers::BlockVerifier;
use mina_tree::verifier::get_srs;

// Re-exported so consumers need not depend on mina-p2p-messages directly.
pub use mina_p2p_messages::v2::{
    MinaBlockBlockStableV2 as Block, MinaBlockHeaderStableV2 as BlockHeader, StateHash,
};

/// Networks with an embedded blockchain verification key.
pub const SUPPORTED_NETWORKS: &[&str] = &["devnet", "mainnet"];

/// Error constructing a [`Verifier`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifierError {
    /// Network name not in [`SUPPORTED_NETWORKS`].
    UnknownNetwork(String),
    /// A different network is already active in this process. Mina's network config
    /// is a process-global set exactly once, so all verifiers in a process must
    /// target the same network.
    NetworkAlreadyInitialized { active: String, requested: String },
}
impl std::fmt::Display for VerifierError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifierError::UnknownNetwork(n) => {
                write!(f, "unknown network {n:?}; supported: {SUPPORTED_NETWORKS:?}")
            }
            VerifierError::NetworkAlreadyInitialized { active, requested } => write!(
                f,
                "network {active:?} already active in this process; cannot build a {requested:?} verifier"
            ),
        }
    }
}
impl std::error::Error for VerifierError {}

static WORKDIR: Once = Once::new();

/// A block verifier bound to one network's blockchain verification key.
pub struct Verifier {
    network: &'static str,
    index: BlockVerifier,
}

impl Verifier {
    /// Build a verifier for `network` ("devnet" or "mainnet"), using that network's
    /// embedded blockchain verification key.
    ///
    /// Mina's network config is a process-global initialized once; constructing a
    /// verifier for a network other than the one already active returns
    /// [`VerifierError::NetworkAlreadyInitialized`].
    pub fn for_network(network: &str) -> Result<Self, VerifierError> {
        if !SUPPORTED_NETWORKS.contains(&network) {
            return Err(VerifierError::UnknownNetwork(network.to_string()));
        }
        // verify_block writes a debug dump on failure; give it a work dir once.
        WORKDIR.call_once(|| mina_core::set_work_dir(std::env::temp_dir()));
        // init() must run before any global() access (global() lazily defaults to
        // devnet). If it's already set, confirm it matches what we asked for.
        if mina_core::NetworkConfig::init(network).is_err() {
            let active = mina_core::NetworkConfig::global().name;
            if active != network {
                return Err(VerifierError::NetworkAlreadyInitialized {
                    active: active.to_string(),
                    requested: network.to_string(),
                });
            }
        }
        let network = mina_core::NetworkConfig::global().name;
        Ok(Self { network, index: BlockVerifier::make() })
    }

    /// Devnet verifier. Panics only if a *different* network is already active —
    /// use [`Verifier::for_network`] to handle that case.
    pub fn devnet() -> Self {
        Self::for_network("devnet").expect("devnet verifier")
    }

    /// Mainnet verifier. See [`Verifier::devnet`] for the panic condition.
    pub fn mainnet() -> Self {
        Self::for_network("mainnet").expect("mainnet verifier")
    }

    /// The network this verifier targets.
    pub fn network(&self) -> &'static str {
        self.network
    }

    /// Verify a block header's blockchain SNARK proof.
    ///
    /// Returns `true` iff the proof is valid for this network — which, by Pickles
    /// recursion, attests every ancestor block as well. Only `header.protocol_state`
    /// and `header.protocol_state_proof` participate; other header fields are ignored.
    pub fn verify_header(&self, header: &BlockHeader) -> bool {
        let srs = get_srs::<Fp>(); // cached globally after first call
        verify_block_proof(header, &self.index, &srs)
    }

    /// Verify a full block (convenience over [`Verifier::verify_header`]).
    pub fn verify_block(&self, block: &Block) -> bool {
        self.verify_header(&block.header)
    }
}

/// Error decoding a consensus-gossip payload into a block.
#[derive(Debug)]
pub enum DecodeError {
    /// Payload shorter than the 8-byte length prefix.
    TooShort,
    /// binprot decoding failed.
    Binprot(binprot::Error),
    /// The gossip message was not a `NewState` (i.e. not a block).
    NotABlock,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::TooShort => write!(f, "payload shorter than 8-byte gossip prefix"),
            DecodeError::Binprot(e) => write!(f, "binprot decode failed: {e}"),
            DecodeError::NotABlock => write!(f, "gossip message is not a NewState (block)"),
        }
    }
}
impl std::error::Error for DecodeError {}
impl From<binprot::Error> for DecodeError {
    fn from(e: binprot::Error) -> Self {
        DecodeError::Binprot(e)
    }
}

/// Decode a Mina consensus-gossip payload into a block.
///
/// The on-wire payload is `[8-byte LE length][GossipNetMessageV2 binprot]`; only a
/// `NewState` message carries a block. The 8-byte prefix is stripped here.
pub fn block_from_gossip_payload(payload: &[u8]) -> Result<Block, DecodeError> {
    if payload.len() < 8 {
        return Err(DecodeError::TooShort);
    }
    let mut cursor = &payload[8..];
    match GossipNetMessageV2::binprot_read(&mut cursor)? {
        GossipNetMessageV2::NewState(block) => Ok((*block).clone()),
        _ => Err(DecodeError::NotABlock),
    }
}

/// Decode a block from raw `MinaBlockBlockStableV2` binprot bytes — for blocks
/// obtained from a source other than gossip (RPC, precomputed, archive).
pub fn block_from_binprot(bytes: &[u8]) -> Result<Block, DecodeError> {
    let mut cursor = bytes;
    Ok(Block::binprot_read(&mut cursor)?)
}
