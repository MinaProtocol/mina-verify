//! Trustless account reads over the sync-ledger RPC — **transport-agnostic**.
//!
//! [`account.rs`](crate::account) is the trust core: given an account + a Merkle path
//! it folds them to the ledger root a verified block commits to. This module is the
//! protocol layer that *obtains* that account + path from an untrusted peer via Mina's
//! `answer_sync_ledger_query` RPC, without trusting the peer — any lie is caught when
//! the assembled path fails to fold to the verified root.
//!
//! It is deliberately transport-free: [`sync_ledger_queries`] returns the RPC *queries*
//! to issue, and [`account_with_path`] / [`verify_account`] consume the *answers*. The
//! caller supplies the wire (libp2p in `mina-light-node`, but equally a mobile app or
//! the MCP server over any transport). This keeps every trust-critical step in one
//! reusable crate instead of re-derived per consumer.
//!
//! ## The walk
//! A Mina account ledger is a depth-[`LEDGER_DEPTH`] binary Merkle tree; an account
//! sits at a leaf identified by its index. To read it:
//! - one `What_child_hashes` query per level (root→leaf): each returns the two child
//!   hashes, of which the **off-path** one is a sibling on the account's Merkle path;
//! - one `What_contents` query at the leaf: returns the account itself.
//!
//! The siblings collected root→leaf are reversed to leaf→root — the order
//! [`crate::implied_root`] folds (height 0 = the account's immediate sibling).

use mina_curves::pasta::Fp;
use mina_p2p_messages::v2::{
    LedgerHash, MerkleAddressBinableArgStableV1, MinaBaseAccountBinableArgStableV2,
    MinaLedgerSyncLedgerAnswerStableV2 as SyncAnswer,
    MinaLedgerSyncLedgerQueryStableV1 as SyncQuery,
};
use mina_tree::{Account, AccountIndex, Address, MerklePath};

use crate::{implied_root, Block};

/// Depth of the Mina account ledger (mainnet / devnet / mesa). An account index is a
/// [`LEDGER_DEPTH`]-bit path from the root; the Merkle path has this many siblings.
pub const LEDGER_DEPTH: usize = 35;

/// Subtree height the live sync-ledger responder serves `What_contents` at: it returns
/// the `2^h` accounts under a subtree rooted at depth `LEDGER_DEPTH - h`. Probed against
/// live devnet — `h = 5` (32 accounts/batch) is served; bigger batches are refused. Used
/// to sweep the whole ledger cheaply (≈ `num_accounts / 32` calls) to build a
/// public-key → leaf-index map (an untrusted hint; every read still re-proves inclusion).
pub const CONTENTS_SUBTREE_HEIGHT: usize = 5;

/// Something went wrong reading an account from sync-ledger answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountReadError {
    /// The answer vector length didn't match the query plan (`depth + 1`).
    AnswerCountMismatch { expected: usize, got: usize },
    /// A `What_child_hashes` slot held something other than `ChildHashesAre`.
    NotChildHashes { level: usize },
    /// The `What_contents` slot held something other than `ContentsAre`, or was empty.
    NotContents,
    /// A ledger hash in an answer wasn't a valid field element.
    BadHash,
    /// The account binprot didn't decode into a ledger account.
    BadAccount,
    /// The account + assembled path did not fold to the verified block's ledger root —
    /// the peer served a wrong account, a wrong path, or a stale ledger.
    NotIncluded,
}

impl std::fmt::Display for AccountReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountReadError::AnswerCountMismatch { expected, got } => {
                write!(f, "expected {expected} sync-ledger answers, got {got}")
            }
            AccountReadError::NotChildHashes { level } => {
                write!(f, "answer at level {level} was not ChildHashesAre")
            }
            AccountReadError::NotContents => {
                write!(f, "leaf answer was not a non-empty ContentsAre")
            }
            AccountReadError::BadHash => {
                write!(f, "a sync-ledger hash was not a valid field element")
            }
            AccountReadError::BadAccount => write!(f, "account binprot did not decode"),
            AccountReadError::NotIncluded => {
                write!(
                    f,
                    "account + path do not fold to the block's verified ledger root"
                )
            }
        }
    }
}
impl std::error::Error for AccountReadError {}

/// The address of the depth-`level` node on the path to leaf `index` (its top `level`
/// bits), as the RPC's Merkle-address argument.
fn node_addr(index: u64, depth: usize, level: usize) -> MerkleAddressBinableArgStableV1 {
    let prefix = index >> (depth - level); // top `level` bits
    Address::from_index(AccountIndex(prefix), level).into()
}

/// The sync-ledger queries to read the account at `index` in a depth-`depth` ledger:
/// one `What_child_hashes` per level root→leaf, then `What_contents` for the leaf.
///
/// Issue each against the verified block's ledger root (the RPC pairs it with that
/// hash) and pass the answers, **in the same order**, to [`account_with_path`].
pub fn sync_ledger_queries(index: u64, depth: usize) -> Vec<SyncQuery> {
    let mut queries = Vec::with_capacity(depth + 1);
    for level in 0..depth {
        queries.push(SyncQuery::WhatChildHashes(node_addr(index, depth, level)));
    }
    let leaf = Address::from_index(AccountIndex(index), depth);
    queries.push(SyncQuery::WhatContents(leaf.into()));
    queries
}

/// The **staking epoch** ledger root from a (verified) block's consensus state — a
/// proven field, and in practice the ledger that live peers actually serve over the
/// sync-ledger RPC (the tip's *staged* root is not served; see [`verify_account_at_root`]).
/// Pair this with [`sync_ledger_queries`] to read finalized, proof-anchored balances.
pub fn staking_epoch_ledger_hash(block: &Block) -> LedgerHash {
    block
        .header
        .protocol_state
        .body
        .consensus_state
        .staking_epoch_data
        .ledger
        .hash
        .clone()
}

/// The **next epoch** ledger root from a (verified) block's consensus state — the other
/// proven, peer-served sync-ledger target (see [`staking_epoch_ledger_hash`]).
pub fn next_epoch_ledger_hash(block: &Block) -> LedgerHash {
    block
        .header
        .protocol_state
        .body
        .consensus_state
        .next_epoch_data
        .ledger
        .hash
        .clone()
}

/// The leaf index where the queries from [`ledger_sweep_queries`]`(start_index, …)` begin
/// — `start_index` rounded down to its `What_contents` subtree boundary. Pass this as the
/// `base_index` to [`pubkey_index_pairs`].
pub fn sweep_base_index(start_index: u64) -> u64 {
    (start_index >> CONTENTS_SUBTREE_HEIGHT) << CONTENTS_SUBTREE_HEIGHT
}

/// Queries to sweep the leaves at indices `[start_index, num_accounts)` of a depth-`depth`
/// ledger: one `What_contents` per subtree at depth `depth - CONTENTS_SUBTREE_HEIGHT`, in
/// index order. `start_index = 0` sweeps the whole ledger; a larger `start_index` sweeps
/// only the appended tail (Mina indices are permanent + append-only, so a pubkey→index map
/// is monotonic and only the growth needs re-sweeping). Feed the answers, with
/// [`sweep_base_index(start_index)`](sweep_base_index), to [`pubkey_index_pairs`].
pub fn ledger_sweep_queries(start_index: u64, num_accounts: u64, depth: usize) -> Vec<SyncQuery> {
    let subtree_depth = depth - CONTENTS_SUBTREE_HEIGHT;
    let first = start_index >> CONTENTS_SUBTREE_HEIGHT;
    let last = num_accounts.div_ceil(1u64 << CONTENTS_SUBTREE_HEIGHT);
    (first..last)
        .map(|s| {
            let addr = Address::from_index(AccountIndex(s), subtree_depth);
            SyncQuery::WhatContents(addr.into())
        })
        .collect()
}

/// Parse the answers to [`ledger_sweep_queries`] (same order) into
/// `(public-key address, leaf index)` pairs. `base_index` is the leaf index of the first
/// answer's subtree — [`sweep_base_index`] of the `start_index` you swept from. Subtree
/// `s`'s batch holds the accounts at leaf indices `base_index + s * 2^h + j` for the
/// `j`-th returned account. The mapping is an **untrusted hint** — a wrong pair can't
/// forge a balance, since [`verify_account_at_root`] re-proves the leaf and the caller
/// cross-checks the pubkey.
pub fn pubkey_index_pairs(
    answers: &[SyncAnswer],
    base_index: u64,
    _depth: usize,
) -> Result<Vec<(String, u64)>, AccountReadError> {
    let batch = 1u64 << CONTENTS_SUBTREE_HEIGHT;
    let mut pairs = Vec::new();
    for (s, answer) in answers.iter().enumerate() {
        match answer {
            SyncAnswer::ContentsAre(accounts) => {
                let subtree_base = base_index + s as u64 * batch;
                for (j, a) in accounts.iter().enumerate() {
                    let account = Account::try_from(a).map_err(|_| AccountReadError::BadAccount)?;
                    pairs.push((account.public_key.into_address(), subtree_base + j as u64));
                }
            }
            _ => return Err(AccountReadError::NotContents),
        }
    }
    Ok(pairs)
}

fn hash_to_fp(h: &LedgerHash) -> Result<Fp, AccountReadError> {
    h.to_field::<Fp>().map_err(|_| AccountReadError::BadHash)
}

/// Assemble the account at `index` and its Merkle path from the `answers` to
/// [`sync_ledger_queries(index, depth)`]. **Does not verify** — fold with
/// [`crate::verify_account_inclusion`] (or call [`verify_account`]) against a verified
/// block before trusting the result.
pub fn account_with_path(
    index: u64,
    depth: usize,
    answers: &[SyncAnswer],
) -> Result<(Account, Vec<MerklePath>), AccountReadError> {
    if answers.len() != depth + 1 {
        return Err(AccountReadError::AnswerCountMismatch {
            expected: depth + 1,
            got: answers.len(),
        });
    }

    // Child-hash answers, root→leaf. At each level the on-path direction is the
    // corresponding bit of `index` (MSB-first); the *other* child is the sibling.
    let mut root_to_leaf = Vec::with_capacity(depth);
    for (level, answer) in answers[..depth].iter().enumerate() {
        let (left, right) = match answer {
            SyncAnswer::ChildHashesAre(l, r) => (l, r),
            _ => return Err(AccountReadError::NotChildHashes { level }),
        };
        let goes_right = (index >> (depth - level - 1)) & 1 == 1;
        root_to_leaf.push(if goes_right {
            MerklePath::Right(hash_to_fp(left)?) // we are the right child; sibling is left
        } else {
            MerklePath::Left(hash_to_fp(right)?) // we are the left child; sibling is right
        });
    }
    root_to_leaf.reverse(); // leaf→root: the order implied_root folds (height 0 first)
    let path = root_to_leaf;

    // Leaf contents: the account at our index. `What_contents` at full leaf depth is a
    // one-account subtree; take the first (and only) account.
    let account = match &answers[depth] {
        SyncAnswer::ContentsAre(accounts) => accounts
            .iter()
            .next()
            .ok_or(AccountReadError::NotContents)
            .and_then(|a: &MinaBaseAccountBinableArgStableV2| {
                Account::try_from(a).map_err(|_| AccountReadError::BadAccount)
            })?,
        _ => return Err(AccountReadError::NotContents),
    };

    Ok((account, path))
}

/// Read **and verify** the account at `index` against an explicit proven ledger `root`:
/// assemble the account + path from `answers`, then confirm they fold to `root`. The
/// root's trustworthiness comes from being a field of an already-verified block — e.g.
/// [`staking_epoch_ledger_hash`], which is what live peers actually serve over the
/// sync-ledger RPC (the tip's staged root is not served). A peer that lies about the
/// account, the path, or the ledger is caught here (`NotIncluded`). Returns the trusted
/// account on success.
pub fn verify_account_at_root(
    root: &LedgerHash,
    index: u64,
    depth: usize,
    answers: &[SyncAnswer],
) -> Result<Account, AccountReadError> {
    let (account, path) = account_with_path(index, depth, answers)?;
    let root_fp = root
        .to_field::<Fp>()
        .map_err(|_| AccountReadError::BadHash)?;
    if implied_root(&account, &path) == root_fp {
        Ok(account)
    } else {
        Err(AccountReadError::NotIncluded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mina_p2p_messages::list::List;
    use mina_p2p_messages::v2::MinaBaseLedgerHash0StableV1;
    use mina_signer::CompressedPubKey;
    use mina_tree::scan_state::currency::Balance;
    use mina_tree::{AccountId, BaseLedger, Database, TokenId, V2};

    // Answer our own queries from a real mina-tree ledger — i.e. act as the peer the
    // light node would talk to — so the test exercises the exact query plan + sibling
    // ordering against mina-tree's own Merkle layout, no live network needed.
    fn serve(db: &mut Database<V2>, depth: usize, query: &SyncQuery) -> SyncAnswer {
        match query {
            SyncQuery::WhatChildHashes(addr) => {
                let node: Address = addr.into();
                let l = db.get_inner_hash_at_addr(node.child_left()).unwrap();
                let r = db.get_inner_hash_at_addr(node.child_right()).unwrap();
                let wrap = |fp: Fp| LedgerHash::from(MinaBaseLedgerHash0StableV1(fp.into()));
                SyncAnswer::ChildHashesAre(wrap(l), wrap(r))
            }
            // WhatContents at any depth: return the filled accounts under the subtree, in
            // leaf-index order (a single account when the address is a full-depth leaf).
            SyncQuery::WhatContents(addr) => {
                let node: Address = addr.into();
                let span = 1u64 << (depth - node.length());
                let base = node.to_index().0 * span;
                let accounts: List<_> = (0..span)
                    .filter_map(|k| {
                        db.get_at_index(AccountIndex(base + k))
                            .map(|a| (&*a).into())
                    })
                    .collect();
                SyncAnswer::ContentsAre(accounts)
            }
            SyncQuery::NumAccounts => unreachable!("not part of an account read"),
        }
    }

    #[test]
    fn reconstructs_mina_tree_path_for_several_accounts() {
        let depth = 10; // small tree; the logic is depth-agnostic
        let mut db = Database::create(depth as u8);

        // A handful of accounts at different leaf positions: one valid public key ×
        // distinct token ids → distinct account ids → distinct leaf indices (good
        // left/right path coverage).
        let pk = CompressedPubKey::from_address(
            "B62qnzbXmRNo9q32n4SNu2mpB8e7FYYLH8NmaX6oFCBYjjQ8SbD7uzV",
        )
        .unwrap();
        let mut ids = Vec::new();
        for token in 1u64..=5 {
            let id = AccountId::new(pk.clone(), TokenId::from(token));
            let acct = Account::create_with(id.clone(), Balance::from_u64(1000 + token));
            db.get_or_create_account(id.clone(), acct).unwrap();
            ids.push(id);
        }

        for id in ids {
            let index = db.index_of_account(id.clone()).unwrap();
            let want_path = db.merkle_path_at_index(index);
            let want_account = *db.get_at_index(index).unwrap();

            // Build the query plan, serve it from the same ledger, assemble back.
            let queries = sync_ledger_queries(index.0, depth);
            assert_eq!(queries.len(), depth + 1);
            let answers: Vec<SyncAnswer> =
                queries.iter().map(|q| serve(&mut db, depth, q)).collect();
            let (got_account, got_path) = account_with_path(index.0, depth, &answers).unwrap();

            assert_eq!(got_path, want_path, "assembled path must match mina-tree's");
            assert_eq!(got_account, want_account, "assembled account must match");
            // And the assembled path must fold to the real root.
            assert_eq!(implied_root_for(&got_account, &got_path), db.merkle_root());

            // The full verify-at-root entry point (incl. LedgerHash conversion) accepts
            // the true root and rejects a wrong one.
            let root = LedgerHash::from(MinaBaseLedgerHash0StableV1(db.merkle_root().into()));
            assert_eq!(
                verify_account_at_root(&root, index.0, depth, &answers).unwrap(),
                want_account
            );
            let wrong = LedgerHash::from(MinaBaseLedgerHash0StableV1(Fp::from(7u64).into()));
            assert_eq!(
                verify_account_at_root(&wrong, index.0, depth, &answers),
                Err(AccountReadError::NotIncluded)
            );
        }
    }

    fn implied_root_for(account: &Account, path: &[MerklePath]) -> Fp {
        crate::implied_root(account, path)
    }

    #[test]
    fn wrong_answer_count_is_rejected() {
        let err = account_with_path(0, 10, &[]).unwrap_err();
        assert_eq!(
            err,
            AccountReadError::AnswerCountMismatch {
                expected: 11,
                got: 0
            }
        );
    }

    #[test]
    fn sweep_reconstructs_every_leaf_index() {
        // Build a ledger with more accounts than one What_contents batch (2^5 = 32) so
        // the sweep spans multiple subtrees, then check the reconstructed leaf indices
        // are exactly 0..n against mina-tree's own indexing.
        let depth = 12;
        let mut db = Database::create(depth as u8);
        let pk = CompressedPubKey::from_address(
            "B62qnzbXmRNo9q32n4SNu2mpB8e7FYYLH8NmaX6oFCBYjjQ8SbD7uzV",
        )
        .unwrap();
        let n: u64 = 70; // > 2 subtrees of 32
        for token in 1..=n {
            let id = AccountId::new(pk.clone(), TokenId::from(token));
            let acct = Account::create_with(id.clone(), Balance::from_u64(token));
            db.get_or_create_account(id, acct).unwrap();
        }

        // Full sweep from 0: every leaf index 0..n.
        let queries = ledger_sweep_queries(0, n, depth);
        assert_eq!(
            queries.len(),
            (n as usize).div_ceil(32),
            "one query per 32-account subtree"
        );
        let answers: Vec<SyncAnswer> = queries.iter().map(|q| serve(&mut db, depth, q)).collect();
        let pairs = pubkey_index_pairs(&answers, sweep_base_index(0), depth).unwrap();
        assert_eq!(pairs.len() as u64, n, "every account is swept exactly once");
        let mut indices: Vec<u64> = pairs.iter().map(|(_, i)| *i).collect();
        indices.sort_unstable();
        assert_eq!(
            indices,
            (0..n).collect::<Vec<_>>(),
            "leaf indices reconstructed exactly"
        );
        let want_addr = pk.into_address();
        assert!(pairs.iter().all(|(addr, _)| addr == &want_addr));

        // Incremental tail sweep from index 40: must reconstruct indices 40..n (rounded
        // down to the subtree boundary, index 32), never re-deriving the wrong index.
        let start = 40;
        let base = sweep_base_index(start);
        assert_eq!(base, 32);
        let tail_q = ledger_sweep_queries(start, n, depth);
        let tail_a: Vec<SyncAnswer> = tail_q.iter().map(|q| serve(&mut db, depth, q)).collect();
        let tail = pubkey_index_pairs(&tail_a, base, depth).unwrap();
        let mut tail_idx: Vec<u64> = tail.iter().map(|(_, i)| *i).collect();
        tail_idx.sort_unstable();
        assert_eq!(
            tail_idx,
            (base..n).collect::<Vec<_>>(),
            "tail sweep reconstructs indices from the subtree boundary"
        );
    }
}
