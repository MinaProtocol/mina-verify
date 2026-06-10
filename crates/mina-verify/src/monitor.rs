//! A windowed chain monitor over *verified* tips.
//!
//! [`crate::fork_choice`] decides between two tips. To actually *track* a chain you
//! also need history: is a new tip simply extending the chain, or is it a competing
//! branch — and if so, where do they diverge? This monitor keeps a bounded window of
//! recently-ingested blocks keyed by state hash, links them by `previous_state_hash`,
//! and classifies each new (already proof-verified) tip relative to the current best.
//!
//! Because Mina gossips every block as it is produced, feeding the gossip stream in
//! makes the window self-sufficient: ancestry of a fork resolves from the window with
//! no extra fetch. A divergence whose common ancestor has fallen out of the window is
//! reported as [`Ingest::Unlinked`] rather than guessed at.

use std::collections::{HashMap, VecDeque};

use mina_core::consensus::consensus_take;
use mina_p2p_messages::v2::ConsensusProofOfStakeDataConsensusStateValueStableV2 as ConsensusState;
use mina_p2p_messages::v2::StateHash;

use crate::VerifiedTip;

struct Node {
    hash: StateHash,
    prev: String,
    height: u32,
    cs: ConsensusState,
}

/// What happened when a verified tip was ingested.
#[derive(Debug, Clone)]
pub enum Ingest {
    /// First block seen — adopted as best.
    Genesis,
    /// Directly extends the current best chain (new best).
    Extend { height: u32 },
    /// Already in the window (same state hash) — ignored.
    Duplicate,
    /// On the best chain but at/below the best (an orphan or an older block).
    Behind { height: u32 },
    /// A reorg: the new tip wins fork-choice over the previous best. The chains
    /// diverge at `common_ancestor`; `depth` is how many blocks were rolled back.
    Reorg {
        from: StateHash,
        common_ancestor: Option<String>,
        depth: Option<u32>,
    },
    /// A competing fork that does NOT win — best is unchanged. Diverges at
    /// `common_ancestor`.
    Fork { common_ancestor: Option<String> },
    /// The tip's relationship to best could not be resolved: its common ancestor
    /// with best has fallen outside the window.
    Unlinked,
}

/// Tracks the canonical chain across a bounded window of verified tips.
pub struct ChainMonitor {
    nodes: HashMap<String, Node>,
    order: VecDeque<String>,
    capacity: usize,
    best: Option<String>,
}

impl ChainMonitor {
    /// Create a monitor retaining up to `capacity` recent blocks (the current best
    /// is never evicted). A capacity around the transition-frontier depth (~290)
    /// comfortably covers any practically-resolvable fork.
    pub fn new(capacity: usize) -> Self {
        Self {
            nodes: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
            best: None,
        }
    }

    /// The current canonical tip's state hash.
    pub fn best(&self) -> Option<&StateHash> {
        self.best
            .as_ref()
            .and_then(|k| self.nodes.get(k))
            .map(|n| &n.hash)
    }

    /// The current canonical tip's height.
    pub fn best_height(&self) -> Option<u32> {
        self.best
            .as_ref()
            .and_then(|k| self.nodes.get(k))
            .map(|n| n.height)
    }

    /// Number of blocks currently in the window.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Is the window empty?
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Ingest a proof-verified tip and classify it against the current best.
    pub fn ingest(&mut self, tip: &VerifiedTip) -> Ingest {
        let key = tip.state_hash().to_string();
        if self.nodes.contains_key(&key) {
            return Ingest::Duplicate;
        }
        let prev = tip
            .block()
            .header
            .protocol_state
            .previous_state_hash
            .to_string();
        let height = tip.height();
        self.insert(
            key.clone(),
            Node {
                hash: tip.state_hash().clone(),
                prev: prev.clone(),
                height,
                cs: tip.consensus_state().clone(),
            },
        );

        let best_key = match self.best.clone() {
            None => {
                self.best = Some(key);
                return Ingest::Genesis;
            }
            Some(b) => b,
        };

        if prev == best_key {
            self.best = Some(key);
            return Ingest::Extend { height };
        }

        // Competing branch: decide by Samasika, then locate the divergence point.
        let (better, best_height, best_hash) = {
            let best_node = self.nodes.get(&best_key).expect("best is in window");
            let cand_node = self.nodes.get(&key).expect("just inserted");
            (
                consensus_take(
                    &best_node.cs,
                    &cand_node.cs,
                    &best_node.hash,
                    &cand_node.hash,
                ),
                best_node.height,
                best_node.hash.clone(),
            )
        };
        let common = self.common_ancestor(&key, &best_key);

        if better {
            self.best = Some(key.clone());
            if common.as_deref() == Some(best_key.as_str()) {
                // best is an ancestor of the new tip -> a multi-block extension.
                Ingest::Extend { height }
            } else {
                let depth = common
                    .as_ref()
                    .and_then(|c| self.nodes.get(c))
                    .map(|c| best_height.saturating_sub(c.height));
                Ingest::Reorg {
                    from: best_hash,
                    common_ancestor: common,
                    depth,
                }
            }
        } else if common.as_deref() == Some(key.as_str()) {
            Ingest::Behind { height } // new tip is an ancestor of best
        } else if common.is_some() {
            Ingest::Fork {
                common_ancestor: common,
            }
        } else {
            Ingest::Unlinked
        }
    }

    /// Lowest-common-ancestor of two in-window blocks by walking parent links.
    /// `None` if the shared history isn't fully in the window.
    fn common_ancestor(&self, a: &str, b: &str) -> Option<String> {
        let mut x = a.to_string();
        let mut y = b.to_string();
        // Bring both to equal height.
        loop {
            let hx = self.nodes.get(&x)?.height;
            let hy = self.nodes.get(&y)?.height;
            if hx == hy {
                break;
            }
            if hx > hy {
                x = self.nodes.get(&x)?.prev.clone();
            } else {
                y = self.nodes.get(&y)?.prev.clone();
            }
        }
        // Step in lockstep until the parents meet.
        while x != y {
            x = self.nodes.get(&x)?.prev.clone();
            y = self.nodes.get(&y)?.prev.clone();
        }
        Some(x)
    }

    fn insert(&mut self, key: String, node: Node) {
        self.nodes.insert(key.clone(), node);
        self.order.push_back(key);
        while self.nodes.len() > self.capacity {
            // Evict the oldest entry that isn't the current best.
            let mut rotated = 0;
            loop {
                match self.order.pop_front() {
                    Some(k) if Some(&k) == self.best.as_ref() => {
                        self.order.push_back(k);
                        rotated += 1;
                        if rotated >= self.order.len() {
                            return; // only the best remains
                        }
                    }
                    Some(k) => {
                        self.nodes.remove(&k);
                        break;
                    }
                    None => return,
                }
            }
        }
    }
}
