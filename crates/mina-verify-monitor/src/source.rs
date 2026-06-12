//! A **block source** for trustless verification: where the block to verify comes
//! from, decoupled from the verification itself. Every variant ultimately yields a
//! single block whose Pickles/kimchi proof is verified, producing the proof-backed
//! [`VerifiedBlock`] facts an indexer (or the SDK) ingests.
//!
//! Two sources today:
//! - [`BlockSource::Precomputed`] — the daemon-published JSON block (GCS / archive).
//!   No networking; the canonical starting point.
//! - [`BlockSource::Node`] — connect to a live Mina node over libp2p and fetch its
//!   best tip via RPC `get_best_tip` (the *seed* path: deterministic, no gossip-mesh
//!   wait). This verifies the live tip directly against an untrusted node.
//!
//! Both go through the same [`Verifier`], so the trust story is identical: the bytes
//! come from an untrusted place, and only a valid proof lets them through.

use std::time::Duration;

use mina_verify::{VerifiedBlock, Verifier, VerifierError};
use mina_verify_capture::{network_seeds, rpc_net};

/// The default time budget for the live-node RPC fetch (dial + handshake + best_tip).
pub const DEFAULT_NODE_DEADLINE: Duration = Duration::from_secs(90);

/// Where a block to verify is obtained from. See the module docs.
pub enum BlockSource {
    /// A precomputed-block JSON (the `{ "version", "data" }` GCS form, or a bare block
    /// object), already read into memory.
    Precomputed(String),
    /// A live Mina node, reached over libp2p using the network's seed peers; its best
    /// tip is fetched via RPC `get_best_tip`.
    Node {
        /// Network name — selects both the seed peers/chain-id and (by convention) the
        /// verifier's VK. One of [`mina_verify_capture::network_seeds`]' keys
        /// ("devnet" / "mainnet" / "mesa-mut").
        network: String,
        /// Time budget for the whole RPC fetch.
        deadline: Duration,
    },
}

impl BlockSource {
    /// A live-node source for `network` with the [`DEFAULT_NODE_DEADLINE`].
    pub fn node(network: impl Into<String>) -> Self {
        BlockSource::Node {
            network: network.into(),
            deadline: DEFAULT_NODE_DEADLINE,
        }
    }
}

/// Why acquiring-and-verifying a block from a [`BlockSource`] failed.
#[derive(Debug)]
pub enum SourceError {
    /// The block was obtained but its proof did not verify, or it couldn't be decoded.
    /// (`VerifierError::ProofInvalid` specifically means: do NOT ingest.)
    Verifier(VerifierError),
    /// A live-node source named a network with no known seed peers.
    UnknownNetwork(String),
    /// The libp2p connect / RPC `get_best_tip` failed (dial, handshake, timeout).
    Rpc(String),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Verifier(e) => write!(f, "{e}"),
            SourceError::UnknownNetwork(n) => {
                write!(
                    f,
                    "no seed peers for network {n:?} (devnet|mainnet|mesa-mut)"
                )
            }
            SourceError::Rpc(e) => write!(f, "node RPC failed: {e}"),
        }
    }
}
impl std::error::Error for SourceError {}
impl From<VerifierError> for SourceError {
    fn from(e: VerifierError) -> Self {
        SourceError::Verifier(e)
    }
}

/// Acquire a block from `source` and verify its proof, returning the proof-backed
/// [`VerifiedBlock`] facts (height, state hash, parent, staged-ledger root).
///
/// `Err` means either acquisition failed ([`SourceError::Rpc`] /
/// [`SourceError::UnknownNetwork`]) or the proof did not verify
/// ([`SourceError::Verifier`]) — in no case should the caller ingest the block.
///
/// The `verifier`'s VK must match the source's network; for a [`BlockSource::Node`]
/// the network is named in the source, so build the verifier for that same network.
pub async fn verify_from(
    verifier: &Verifier,
    source: BlockSource,
) -> Result<VerifiedBlock, SourceError> {
    match source {
        BlockSource::Precomputed(json) => Ok(verifier.verify_precomputed_and_extract(&json)?),
        BlockSource::Node { network, deadline } => {
            let (chain_id, peers) =
                network_seeds(&network).ok_or(SourceError::UnknownNetwork(network))?;
            let block = rpc_net::fetch_best_tip(chain_id, peers, deadline)
                .await
                .map_err(SourceError::Rpc)?;
            Ok(verifier.verify_and_extract(&block.header)?)
        }
    }
}
