//! Verifier layer for a **trustless indexer**.
//!
//! An indexer that wants every row it serves to be proof-backed does *verify-before-ingest*:
//! for each block, verify its SNARK proof; if it fails, reject (never persist). Then, after
//! the indexer replays the block's transactions into its own ledger, it checks that ledger's
//! root against the proof-backed [`VerifiedBlock::staged_ledger_hash`] — a mismatch means the
//! indexer's state diverged from the chain (a bug, or a poisoned input). With both checks,
//! the indexer's whole GraphQL is trustworthy without per-account Merkle proofs.
//!
//! ```ignore
//! let verifier = Verifier::for_network("devnet")?;
//! // ingest loop (precomputed blocks from GCS, or live):
//! let vb = verifier.verify_precomputed_and_extract(&json)?;   // Err => reject, don't ingest
//! // … indexer replays vb's transactions, computes its own staged-ledger root …
//! if computed_root != vb.staged_ledger_hash {                  // diverged — halt/alert
//!     return Err(/* poisoned or buggy */);
//! }
//! // persist, keyed by vb.state_hash / vb.height, with vb.previous_state_hash for linkage.
//! ```
//!
//! **Optimization — you don't have to verify every block.** By Pickles recursion, verifying
//! one block's proof attests every ancestor. So an indexer can verify the *tip* (and re-verify
//! periodically as it advances) and rely on `previous_state_hash` linkage plus the per-block
//! staged-ledger-hash check for the blocks in between, instead of re-proving deep history.

use mina_p2p_messages::v2::LedgerHash;

use crate::{header_from_precomputed, BlockHeader, StateHash, Verifier, VerifierError};

/// Proof-backed facts extracted from a verified block — what an indexer needs to ingest
/// trustlessly. Produced only *after* the block's proof verifies, so every field is attested
/// by that proof. Hashes are the typed `mina-p2p-messages` values for exact comparison; format
/// them however the indexer needs.
#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedBlock {
    /// Blockchain length (block height).
    pub height: u32,
    /// This block's state hash — its identity.
    pub state_hash: StateHash,
    /// Parent's state hash — for chain linkage / fork detection.
    pub previous_state_hash: StateHash,
    /// Merkle root of the staged (current-balance) ledger. After replaying this block's
    /// transactions, an indexer's own ledger root MUST equal this.
    pub staged_ledger_hash: LedgerHash,
}

impl VerifiedBlock {
    fn from_header(h: &BlockHeader) -> Result<Self, VerifierError> {
        let cs = &h.protocol_state.body.consensus_state;
        let bs = &h.protocol_state.body.blockchain_state;
        Ok(VerifiedBlock {
            height: cs.blockchain_length.as_u32(),
            state_hash: h
                .try_hash()
                .map_err(|e| VerifierError::BlockDecode(format!("{e:?}")))?,
            previous_state_hash: h.protocol_state.previous_state_hash.clone(),
            staged_ledger_hash: bs.staged_ledger_hash.non_snark.ledger_hash.clone(),
        })
    }
}

impl Verifier {
    /// Verify a block header's proof and return its proof-backed facts.
    /// `Err(VerifierError::ProofInvalid)` if the proof doesn't check — the block must NOT be
    /// ingested.
    pub fn verify_and_extract(&self, header: &BlockHeader) -> Result<VerifiedBlock, VerifierError> {
        if !self.verify_header(header) {
            return Err(VerifierError::ProofInvalid);
        }
        VerifiedBlock::from_header(header)
    }

    /// Decode a precomputed block (the GCS JSON) and [`verify_and_extract`] it — the typical
    /// indexer ingest call.
    ///
    /// [`verify_and_extract`]: Verifier::verify_and_extract
    pub fn verify_precomputed_and_extract(
        &self,
        json: &str,
    ) -> Result<VerifiedBlock, VerifierError> {
        self.verify_and_extract(&header_from_precomputed(json)?)
    }
}
