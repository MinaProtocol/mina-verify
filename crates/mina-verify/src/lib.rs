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

pub use fork_choice::{
    compare_tips, is_short_range_fork, prefers_candidate, select_canonical, ForkRange,
    TipComparison, VerifiedTip,
};

use std::sync::Once;

use binprot::BinProtRead;
use mina_curves::pasta::Fp;
use mina_p2p_messages::gossip::GossipNetMessageV2;
use mina_p2p_messages::v2::{MinaBlockBlockStableV2, MinaBlockHeaderStableV2};
use mina_tree::proofs::verification::verify_block as verify_block_proof;
use mina_tree::proofs::verifiers::BlockVerifier;
use mina_tree::verifier::get_srs;

static INIT: Once = Once::new();

/// A block verifier bound to a network's blockchain verification key.
pub struct Verifier {
    index: BlockVerifier,
}

impl Verifier {
    /// Build a verifier for **devnet**, using the embedded devnet blockchain
    /// verification key. Idempotent: the underlying global network config and
    /// work dir are initialized at most once per process.
    pub fn devnet() -> Self {
        INIT.call_once(|| {
            // OnceCell-backed globals in mina-core; set exactly once.
            let _ = mina_core::NetworkConfig::init("devnet");
            mina_core::set_work_dir(std::env::temp_dir());
        });
        Self {
            index: BlockVerifier::make(),
        }
    }

    /// Verify a block header's blockchain SNARK proof.
    ///
    /// Returns `true` iff the proof is valid for this network — which, by Pickles
    /// recursion, attests every ancestor block as well. Only `header.protocol_state`
    /// and `header.protocol_state_proof` participate; other header fields are ignored.
    pub fn verify_header(&self, header: &MinaBlockHeaderStableV2) -> bool {
        let srs = get_srs::<Fp>(); // cached globally after first call
        verify_block_proof(header, &self.index, &srs)
    }

    /// Verify a full block (convenience over [`Verifier::verify_header`]).
    pub fn verify_block(&self, block: &MinaBlockBlockStableV2) -> bool {
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
pub fn block_from_gossip_payload(payload: &[u8]) -> Result<MinaBlockBlockStableV2, DecodeError> {
    if payload.len() < 8 {
        return Err(DecodeError::TooShort);
    }
    let mut cursor = &payload[8..];
    match GossipNetMessageV2::binprot_read(&mut cursor)? {
        GossipNetMessageV2::NewState(block) => Ok((*block).clone()),
        _ => Err(DecodeError::NotABlock),
    }
}
