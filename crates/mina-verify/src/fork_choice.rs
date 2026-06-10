//! Consensus fork-choice over *verified* tips (Ouroboros Samasika).
//!
//! Single-tip proof verification ([`crate::Verifier::verify_block`]) tells you a
//! block is valid. To run a monitor you also need to decide, between two valid but
//! competing tips (e.g. reported by two different nodes), which one the protocol
//! considers canonical — and to notice when they diverge at all. That selection is
//! Mina's Ouroboros Samasika rule, implemented in `mina-core`; this module wraps it
//! around proof-verified tips so the comparison can never be fed an unverified block.

use mina_core::consensus::{consensus_take, is_short_range_fork as core_is_short_range_fork};
use mina_p2p_messages::bigint::InvalidBigInt;
use mina_p2p_messages::v2::{
    ConsensusProofOfStakeDataConsensusStateValueStableV2 as ConsensusState, MinaBlockBlockStableV2,
    StateHash,
};

use crate::Verifier;

/// A block whose blockchain SNARK proof has been verified, bound to its state hash.
///
/// Constructed only via [`Verifier::verify_tip`], so a `VerifiedTip` is proof of
/// "this block is valid for the network" — fork-choice never sees an unverified tip.
#[derive(Clone)]
pub struct VerifiedTip {
    block: MinaBlockBlockStableV2,
    state_hash: StateHash,
}

impl VerifiedTip {
    /// The full verified block.
    pub fn block(&self) -> &MinaBlockBlockStableV2 {
        &self.block
    }
    /// The block's state hash (computed, not trusted from the source).
    pub fn state_hash(&self) -> &StateHash {
        &self.state_hash
    }
    /// The consensus state used by fork-choice.
    pub fn consensus_state(&self) -> &ConsensusState {
        &self.block.header.protocol_state.body.consensus_state
    }
    /// Blockchain length (height).
    pub fn height(&self) -> u32 {
        self.consensus_state().blockchain_length.as_u32()
    }
}

impl Verifier {
    /// Verify a block's proof and, if valid, bind it to its computed state hash.
    ///
    /// Returns `Ok(None)` if the proof does not verify, `Ok(Some(tip))` if it does,
    /// and `Err` only if the block contains a malformed field (invalid bigint).
    pub fn verify_tip(
        &self,
        block: MinaBlockBlockStableV2,
    ) -> Result<Option<VerifiedTip>, InvalidBigInt> {
        if !self.verify_block(&block) {
            return Ok(None);
        }
        let state_hash = block.try_hash()?;
        Ok(Some(VerifiedTip { block, state_hash }))
    }
}

/// Whether `a` and `b` are a short-range fork (recent common ancestor) as opposed
/// to a long-range one (decided by sub-window density rather than length).
pub fn is_short_range_fork(a: &VerifiedTip, b: &VerifiedTip) -> bool {
    core_is_short_range_fork(a.consensus_state(), b.consensus_state())
}

/// Ouroboros Samasika fork-choice: should `candidate` be adopted over `tip`?
///
/// `true` ⇒ candidate is the better chain. Internally picks short- vs long-range
/// rules and tiebreaks by chain length, sub-window density, VRF, then state hash.
pub fn prefers_candidate(tip: &VerifiedTip, candidate: &VerifiedTip) -> bool {
    consensus_take(
        tip.consensus_state(),
        candidate.consensus_state(),
        &tip.state_hash,
        &candidate.state_hash,
    )
}

/// Short- vs long-range classification of a divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkRange {
    Short,
    Long,
}

/// The result of comparing two verified tips.
#[derive(Debug, Clone)]
pub enum TipComparison {
    /// Identical tip — the two sources agree.
    Same,
    /// The tips differ. `canonical_is_b` is the Samasika winner; `range` says
    /// whether this is a short- or long-range fork.
    ///
    /// NOTE: this reports *divergence at the tip*. Distinguishing "b simply extends
    /// a" from "a genuine competing fork" requires the chain of blocks between them
    /// (the windowed history); that is the next increment (see crate docs).
    Diverged {
        canonical_is_b: bool,
        range: ForkRange,
    },
}

/// Compare two verified tips (e.g. from two different nodes / an indexer vs a node).
pub fn compare_tips(a: &VerifiedTip, b: &VerifiedTip) -> TipComparison {
    if a.state_hash == b.state_hash {
        return TipComparison::Same;
    }
    let range = if is_short_range_fork(a, b) {
        ForkRange::Short
    } else {
        ForkRange::Long
    };
    TipComparison::Diverged {
        canonical_is_b: prefers_candidate(a, b),
        range,
    }
}

/// Select the canonical tip among several verified tips by Samasika fork-choice.
/// Returns the index of the winner, or `None` if `tips` is empty.
pub fn select_canonical(tips: &[VerifiedTip]) -> Option<usize> {
    if tips.is_empty() {
        return None;
    }
    let mut best = 0;
    for i in 1..tips.len() {
        if prefers_candidate(&tips[best], &tips[i]) {
            best = i;
        }
    }
    Some(best)
}
