//! Trustless verification of Mina blocks.
//!
//! Given block bytes obtained from an *untrusted* source (a node, an indexer, a
//! captured gossip message), verify the block's Pickles/kimchi blockchain SNARK
//! proof against the network verification key. If the proof verifies, the entire
//! chain history up to that block is valid by Pickles recursion — no trust in the
//! source is required.
//!
//! ```no_run
//! # let payload: Vec<u8> = Vec::new(); // a captured gossip payload
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

use std::sync::Arc;

use binprot::{BinProtRead, BinProtWrite};
use mina_curves::pasta::{Fp, Fq};
use mina_p2p_messages::gossip::GossipNetMessageV2;
use mina_tree::proofs::verification::verify_block as verify_block_proof;
// On wasm `BlockVerifier::make()` is async; the wasm path uses the embedded VK JSON
// instead (see `for_network`), so this is only needed off-wasm.
#[cfg(not(target_family = "wasm"))]
use mina_tree::proofs::verifiers::BlockVerifier;
use mina_tree::proofs::VerifierIndex;
use mina_tree::verifier::get_srs;

pub mod verifier_index;
pub use verifier_index::verifier_index_from_json;

pub mod account;
pub use account::{implied_root, ledger_root, verify_account_inclusion};

pub mod account_read;
pub use account_read::{
    account_with_path, next_epoch_ledger_hash, staking_epoch_ledger_hash, sync_ledger_queries,
    verify_account_at_root, AccountReadError, LEDGER_DEPTH,
};

pub mod precomputed;
pub use precomputed::header_from_precomputed;

pub mod ingest;
pub use ingest::VerifiedBlock;
/// Account + Merkle-path types for trustless state reads (re-exported from mina-tree).
pub use mina_tree::{Account, MerklePath};

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
    /// The network's embedded blockchain verification key could not be loaded.
    /// (Known: the mainnet verifier index in mina-rust@ab69eaed is in a stale JSON
    /// format and must be regenerated upstream.)
    VerificationKeyUnavailable { network: String },
    /// A caller-supplied verifier-index JSON failed to parse.
    InvalidIndexJson(String),
    /// The block's SNARK proof did not verify — it must not be ingested.
    ProofInvalid,
    /// A block could not be decoded / its fields read.
    BlockDecode(String),
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
            VerifierError::VerificationKeyUnavailable { network } => write!(
                f,
                "could not load the {network:?} blockchain verification key (embedded index unparseable — regenerate it upstream)"
            ),
            VerifierError::InvalidIndexJson(e) => write!(f, "invalid verifier-index JSON: {e}"),
            VerifierError::ProofInvalid => write!(f, "block proof did not verify"),
            VerifierError::BlockDecode(e) => write!(f, "could not decode block: {e}"),
        }
    }
}
impl std::error::Error for VerifierError {}

static WORKDIR: Once = Once::new();

/// Scratch dir mina-tree uses only for failure debug dumps. `std::env::temp_dir()`
/// panics on wasm ("no filesystem on this platform"), so use a placeholder path there
/// — verification never actually writes unless a proof fails, and on wasm a failing
/// proof just won't produce a dump.
fn default_work_dir() -> std::path::PathBuf {
    #[cfg(target_family = "wasm")]
    {
        std::path::PathBuf::from("/tmp")
    }
    #[cfg(not(target_family = "wasm"))]
    {
        std::env::temp_dir()
    }
}

/// A block verifier bound to one network's blockchain verification key.
pub struct Verifier {
    network: String,
    index: Arc<VerifierIndex<Fq>>,
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
        WORKDIR.call_once(|| mina_core::set_work_dir(default_work_dir()));
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
        // mina-tree's embedded mainnet index is in a stale serialization format (old
        // ark byte-arrays, no zk_rows) that its own loader can't parse. Ship our
        // format-migrated mainnet VK and use it instead. (Verified: a live mainnet tip
        // verifies true against it.)
        if network == "mainnet" {
            let mut v =
                Self::with_index_json(include_str!("data/mainnet_blockchain_verifier_index.json"))?;
            v.network = network.to_string();
            return Ok(v);
        }
        // devnet: mina-tree's embedded index is current-format; BlockVerifier::make()
        // parses it and panics (unwrap) if it can't. Catch that and turn it into a
        // clean error so consumers don't crash.
        #[cfg(not(target_family = "wasm"))]
        let index: Arc<VerifierIndex<Fq>> = {
            let prev = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let r = std::panic::catch_unwind(BlockVerifier::make);
            std::panic::set_hook(prev);
            r.map_err(|_| VerifierError::VerificationKeyUnavailable {
                network: network.to_string(),
            })?
            .into()
        };
        // wasm: `BlockVerifier::make()` is async and `catch_unwind` is a no-op under
        // panic=abort. Use the embedded devnet VK JSON instead — same key, parsed
        // synchronously, no global mina-tree state required.
        #[cfg(target_family = "wasm")]
        let index: Arc<VerifierIndex<Fq>> = {
            let json = Self::embedded_index_json("devnet").ok_or_else(|| {
                VerifierError::VerificationKeyUnavailable {
                    network: network.to_string(),
                }
            })?;
            Arc::new(
                verifier_index::verifier_index_from_json(json)
                    .map_err(|e| VerifierError::InvalidIndexJson(e.to_string()))?,
            )
        };
        Ok(Self {
            network: network.to_string(),
            index,
        })
    }

    /// Build a verifier from a caller-supplied blockchain verifier-index JSON (the
    /// `*_blockchain_verifier_index.json` format). Use this for networks whose VK is
    /// not embedded in mina-tree (mesa-mut, future hardforks) or to override a stale
    /// embedded one (mainnet). No global network config is required.
    pub fn with_index_json(json: &str) -> Result<Self, VerifierError> {
        WORKDIR.call_once(|| mina_core::set_work_dir(default_work_dir()));
        let index = verifier_index::verifier_index_from_json(json)
            .map_err(|e| VerifierError::InvalidIndexJson(e.to_string()))?;
        Ok(Self {
            network: "custom".to_string(),
            index: Arc::new(index),
        })
    }

    /// The embedded blockchain verifier-index JSON for a network, if shipped.
    pub fn embedded_index_json(network: &str) -> Option<&'static str> {
        match network {
            "devnet" => Some(include_str!("data/devnet_blockchain_verifier_index.json")),
            "mainnet" => Some(include_str!("data/mainnet_blockchain_verifier_index.json")),
            _ => None,
        }
    }

    /// Build a verifier for `network` from its embedded VK **without** touching the
    /// process-global `NetworkConfig` (which can only be set once). This lets a single
    /// process verify multiple networks — e.g. a mobile app switching devnet/mainnet.
    /// Proof verification is VK-based and config-independent.
    pub fn for_network_offline(network: &str) -> Result<Self, VerifierError> {
        let json = Self::embedded_index_json(network)
            .ok_or_else(|| VerifierError::UnknownNetwork(network.to_string()))?;
        let mut v = Self::with_index_json(json)?;
        v.network = network.to_string();
        Ok(v)
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

    /// The network this verifier targets ("devnet"/"mainnet", or "custom" for
    /// [`Verifier::with_index_json`]).
    pub fn network(&self) -> &str {
        &self.network
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

/// Encode a block to `MinaBlockBlockStableV2` binprot bytes — the inverse of
/// [`block_from_binprot`], e.g. to hand a fetched block across an FFI boundary.
pub fn block_to_binprot(block: &Block) -> Vec<u8> {
    let mut bytes = Vec::new();
    block
        .binprot_write(&mut bytes)
        .expect("binprot_write to a Vec is infallible");
    bytes
}
