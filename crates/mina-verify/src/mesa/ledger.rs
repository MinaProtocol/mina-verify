//! A mesa account ledger: Merkle root and inclusion paths.
//!
//! `mina-tree`'s `Database`/`Mask` give you a ledger, but only for `V2` -- `BaseLedger`
//! is implemented solely for `DatabaseImpl<V2>` and `Mask` is hardcoded to it, so neither
//! can hold a [`MesaAccount`]. The tree itself is small, so we build it here.
//!
//! The *node* hashing is identical to V2 (`poseidon` keyed by the node's height); only the
//! *leaf* hashing is mesa-specific. That is why a mesa inclusion path still folds with
//! `V2::hash_node` -- see [`implied_root`].

use super::account::{AccountOf, MesaAccount};
use mina_tree::MerklePath;
use mina_curves::pasta::Fp;
use once_cell::sync::Lazy;
use poseidon::hash::{hash_with_kimchi, params::get_merkle_param_for_height};

/// Mina's account-ledger depth. Unchanged by mesa.
pub const MESA_LEDGER_DEPTH: usize = 35;

/// Combine two nodes whose children sit at `height`. Identical to `V2::hash_node` --
/// node hashing is not what mesa changed.
pub fn hash_node(height: usize, left: Fp, right: Fp) -> Fp {
    hash_with_kimchi(get_merkle_param_for_height(height), &[left, right])
}

/// The hash of an entirely empty subtree of the given height. `height == 0` is the empty
/// *account*, which is where mesa diverges from V2 -- so these differ from mina-tree's.
pub fn empty_hash_at_height(height: usize) -> Fp {
    static EMPTY: Lazy<Vec<Fp>> = Lazy::new(|| {
        let mut hashes = Vec::with_capacity(MESA_LEDGER_DEPTH + 1);
        hashes.push(MesaAccount::empty().hash());

        for height in 0..MESA_LEDGER_DEPTH {
            let prev = hashes[height];
            hashes.push(hash_node(height, prev, prev));
        }

        hashes
    });

    EMPTY[height]
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
    num_accounts: usize,
}

impl MesaLedger {
    /// Hash every account, then fold the tree up to the root.
    pub fn new<const N: usize>(accounts: &[AccountOf<N>]) -> Self {
        Self::from_leaves_with_empty(
            accounts.iter().map(AccountOf::<N>::hash).collect(),
            AccountOf::<N>::empty().hash(),
        )
    }

    /// As [`MesaLedger::new`], but taking leaf hashes directly -- lets a caller hash the
    /// accounts in parallel, which dominates the cost on a real ledger.
    pub fn from_leaves(leaves: Vec<Fp>) -> Self {
        Self::from_leaves_with_empty(leaves, MesaAccount::empty().hash())
    }

    /// The general form: `empty_leaf` is the hash of the account that fills every unused
    /// slot. For mesa that is [`MesaAccount::empty`]; pass mina-tree's `Account::empty()`
    /// hash to build a V2 tree with this same code.
    pub fn from_leaves_with_empty(leaves: Vec<Fp>, empty_leaf: Fp) -> Self {
        let num_accounts = leaves.len();

        let mut empties = Vec::with_capacity(MESA_LEDGER_DEPTH + 1);
        empties.push(empty_leaf);

        for height in 0..MESA_LEDGER_DEPTH {
            let prev = empties[height];
            empties.push(hash_node(height, prev, prev));
        }

        let mut levels = Vec::with_capacity(MESA_LEDGER_DEPTH + 1);
        let mut current = leaves;

        levels.push(current.clone());

        for height in 0..MESA_LEDGER_DEPTH {
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
            num_accounts,
        }
    }

    pub fn num_accounts(&self) -> usize {
        self.num_accounts
    }

    /// The ledger hash the protocol commits to.
    pub fn merkle_root(&self) -> Fp {
        // an empty ledger is an empty tree; otherwise the top level holds exactly the root
        self.levels[MESA_LEDGER_DEPTH]
            .first()
            .copied()
            .unwrap_or(self.empties[MESA_LEDGER_DEPTH])
    }

    /// The inclusion path for the account at `index`, bottom-up. Folds to
    /// [`MesaLedger::merkle_root`] via [`implied_root`].
    pub fn merkle_path(&self, index: usize) -> Vec<MerklePath> {
        let mut path = Vec::with_capacity(MESA_LEDGER_DEPTH);
        let mut index = index;

        for height in 0..MESA_LEDGER_DEPTH {
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
pub fn implied_root(account: &MesaAccount, merkle_path: &[MerklePath]) -> Fp {
    merkle_path
        .iter()
        .enumerate()
        .fold(account.hash(), |accum, (height, path)| match path {
            MerklePath::Left(right) => hash_node(height, accum, *right),
            MerklePath::Right(left) => hash_node(height, *left, accum),
        })
}

/// `true` iff `account` with `merkle_path` is included in the ledger with `root`.
pub fn verify_account_inclusion(account: &MesaAccount, merkle_path: &[MerklePath], root: Fp) -> bool {
    implied_root(account, merkle_path) == root
}
