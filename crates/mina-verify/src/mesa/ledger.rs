//! A mesa account ledger: Merkle root and inclusion paths.
//!
//! `mina-tree`'s `Database`/`Mask` give you a ledger, but only for `V2` -- `BaseLedger`
//! is implemented solely for `DatabaseImpl<V2>` and `Mask` is hardcoded to it, so neither
//! can hold a [`MesaAccount`]. The tree itself is small, so we build it here.
//!
//! The *node* hashing is identical to V2 (`poseidon` keyed by the node's height); only the
//! *leaf* hashing is mesa-specific. That is why a mesa inclusion path still folds with
//! `V2::hash_node` -- see [`implied_root`].

use super::account::{AccountOf, LedgerParams, MESA};
use mina_curves::pasta::Fp;
use mina_tree::MerklePath;
use poseidon::hash::{hash_with_kimchi, params::get_merkle_param_for_height};

/// Mina's account-ledger depth on both networks we have pinned. Not assumed anywhere --
/// it is carried in [`LedgerParams`], because a build can and does differ (the
/// devnet-*generic* image reports 10).
pub const MESA_LEDGER_DEPTH: usize = MESA.depth;

/// Combine two nodes whose children sit at `height`. Identical to `V2::hash_node` --
/// node hashing is not what mesa changed.
pub fn hash_node(height: usize, left: Fp, right: Fp) -> Fp {
    hash_with_kimchi(get_merkle_param_for_height(height), &[left, right])
}

/// The empty-subtree hash at each height, given the hash of the empty account. Height 0
/// *is* the empty account -- which is where mesa diverges from V2, and where an unpinned
/// transaction version does its damage.
fn empty_hashes(empty_leaf: Fp, depth: usize) -> Vec<Fp> {
    let mut hashes = Vec::with_capacity(depth + 1);
    hashes.push(empty_leaf);

    for height in 0..depth {
        let prev = hashes[height];
        hashes.push(hash_node(height, prev, prev));
    }

    hashes
}

/// A mesa ledger, held as its Merkle levels: `levels[0]` is the account (leaf) hashes,
/// `levels[h]` the nodes at height `h`. Slots past the last account are empty, so a level
/// is only as wide as it needs to be and the empty tail is folded in via
/// [`empty_hash_at_height`].
///
/// Accounts are indexed in ledger order, which for a genesis state dump is the order they
/// appear in the file.
pub struct MesaLedger {
    levels: Vec<Vec<Fp>>,
    /// Hash of an empty subtree at each height. Carried rather than looked up globally so
    /// the tree can be exercised against a *V2* ledger, whose empty account differs --
    /// which is how we test this tree independently of mesa account hashing.
    empties: Vec<Fp>,
    depth: usize,
    num_accounts: usize,
}

impl MesaLedger {
    /// Hash every account, then fold the tree up to the root.
    /// `params` must be the ones pinned for this network -- see [`LedgerParams`]. `N` must
    /// equal `params.zkapp_state_size`.
    pub fn new<const N: usize>(accounts: &[AccountOf<N>], params: LedgerParams) -> Self {
        assert_eq!(
            N, params.zkapp_state_size,
            "account width does not match the network's ledger params"
        );

        Self::from_leaves_with_empty(
            accounts.iter().map(AccountOf::<N>::hash).collect(),
            AccountOf::<N>::empty_with_txn_version(params.txn_version).hash(),
            params.depth,
        )
    }

    /// The general form: `empty_leaf` is the hash of the account that fills every unused
    /// slot. Pass mina-tree's `Account::empty()` hash to build a V2 tree with this code --
    /// which is how the tree is tested against mina-tree itself.
    pub fn from_leaves_with_empty(leaves: Vec<Fp>, empty_leaf: Fp, depth: usize) -> Self {
        let num_accounts = leaves.len();
        let empties = empty_hashes(empty_leaf, depth);

        let mut levels = Vec::with_capacity(depth + 1);
        let mut current = leaves;

        levels.push(current.clone());

        for height in 0..depth {
            let empty = empties[height];
            let mut next = Vec::with_capacity(current.len().div_ceil(2));

            for pair in current.chunks(2) {
                let left = pair[0];
                let right = pair.get(1).copied().unwrap_or(empty);

                next.push(hash_node(height, left, right));
            }

            current = next;
            levels.push(current.clone());
        }

        Self {
            levels,
            empties,
            depth,
            num_accounts,
        }
    }

    pub fn num_accounts(&self) -> usize {
        self.num_accounts
    }

    /// The ledger hash the protocol commits to.
    pub fn merkle_root(&self) -> Fp {
        // an empty ledger is an empty tree; otherwise the top level holds exactly the root
        self.levels[self.depth]
            .first()
            .copied()
            .unwrap_or(self.empties[self.depth])
    }

    /// The inclusion path for the account at `index`, bottom-up. Folds to
    /// [`MesaLedger::merkle_root`] via [`implied_root`].
    pub fn merkle_path(&self, index: usize) -> Vec<MerklePath> {
        let mut path = Vec::with_capacity(self.depth);
        let mut index = index;

        for height in 0..self.depth {
            let sibling = self.levels[height]
                .get(index ^ 1)
                .copied()
                .unwrap_or(self.empties[height]);

            // Left(sibling) == "we are the left child, this is our right sibling"
            path.push(if index % 2 == 0 {
                MerklePath::Left(sibling)
            } else {
                MerklePath::Right(sibling)
            });

            index /= 2;
        }

        path
    }
}

/// Fold a mesa account and its inclusion path to the ledger root it implies. The mesa twin
/// of `mina_verify::implied_root`: same node hashing, mesa leaf hashing.
///
/// A lying source cannot forge this -- the account and path must hash up to the root the
/// block committed to.
pub fn implied_root<const N: usize>(account: &AccountOf<N>, merkle_path: &[MerklePath]) -> Fp {
    merkle_path
        .iter()
        .enumerate()
        .fold(account.hash(), |accum, (height, path)| match path {
            MerklePath::Left(right) => hash_node(height, accum, *right),
            MerklePath::Right(left) => hash_node(height, *left, accum),
        })
}

/// `true` iff `account` with `merkle_path` is included in the ledger with `root`.
pub fn verify_account_inclusion<const N: usize>(
    account: &AccountOf<N>,
    merkle_path: &[MerklePath],
    root: Fp,
) -> bool {
    implied_root(account, merkle_path) == root
}
