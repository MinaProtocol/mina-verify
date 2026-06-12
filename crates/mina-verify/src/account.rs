//! Trustless account / state reads.
//!
//! A verified block commits — via its Pickles/kimchi proof — to the account-ledger Merkle
//! root: the staged ledger hash inside its protocol state. Given an account plus a Merkle
//! inclusion path from an **untrusted** source (a node's RPC, an indexer), this module
//! checks that the path folds to that root. If it does, the account state is trustworthy
//! even though the data came from a source we don't trust — which is exactly what lets a
//! light client read balances / zkApp state without running or trusting a full node.
//!
//! The fold reproduces mina-tree's (private) `verify_merkle_path` using the public
//! [`mina_tree::V2::hash_node`], so it agrees bit-for-bit with how the protocol hashes the
//! ledger.

use mina_curves::pasta::Fp;
use mina_tree::{Account, MerklePath, TreeVersion, V2};

use crate::Block;

/// The account-ledger Merkle root committed to by a (verified) block — the staged ledger
/// hash in its protocol state. `None` if the field can't be read as a field element.
pub fn ledger_root(block: &Block) -> Option<Fp> {
    block
        .header
        .protocol_state
        .body
        .blockchain_state
        .staged_ledger_hash
        .non_snark
        .ledger_hash
        .to_field::<Fp>()
        .ok()
}

/// Fold an account and its Merkle inclusion path to the implied ledger root. Mirrors
/// mina-tree's non-circuit `verify_merkle_path`: leaf = `account.hash()`, then at each
/// level the sibling from the path is combined with the running hash (left/right per the
/// path element) using the height-parameterised node hash.
pub fn implied_root(account: &Account, merkle_path: &[MerklePath]) -> Fp {
    merkle_path
        .iter()
        .enumerate()
        .fold(account.hash(), |accum, (height, path)| match path {
            MerklePath::Left(right) => V2::hash_node(height, accum, *right),
            MerklePath::Right(left) => V2::hash_node(height, *left, accum),
        })
}

/// `true` iff `account` with `merkle_path` is included in the ledger committed to by the
/// (already-verified) `block`. A lying source can't forge this: the account + path must
/// hash up to the block's ledger root, which the block's SNARK proof attests.
pub fn verify_account_inclusion(
    block: &Block,
    account: &Account,
    merkle_path: &[MerklePath],
) -> bool {
    match ledger_root(block) {
        Some(root) => implied_root(account, merkle_path) == root,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mina_signer::CompressedPubKey;
    use mina_tree::scan_state::currency::Balance;
    use mina_tree::{Account, AccountId, BaseLedger, Database, TokenId};

    #[test]
    fn implied_root_matches_real_ledger_and_rejects_tampering() {
        // Build a real ledger with mina-tree's own Merkle implementation, get a genuine
        // inclusion path, and confirm our fold reproduces its root.
        let mut db = Database::create(20);
        let pk = CompressedPubKey::from_address(
            "B62qnzbXmRNo9q32n4SNu2mpB8e7FYYLH8NmaX6oFCBYjjQ8SbD7uzV",
        )
        .unwrap();
        let id = AccountId::new(pk, TokenId::default());
        let account = Account::create_with(id.clone(), Balance::from_u64(10101));
        db.get_or_create_account(id.clone(), account.clone())
            .unwrap();

        let root = db.merkle_root();
        let index = db.index_of_account(id).unwrap();
        let path = db.merkle_path_at_index(index);

        assert_eq!(
            implied_root(&account, &path),
            root,
            "a real account + path must fold to the ledger root"
        );

        // An untrusted source lying about one sibling must NOT verify.
        let tampered: Vec<MerklePath> = path
            .iter()
            .enumerate()
            .map(|(i, p)| match (i, p) {
                (0, MerklePath::Left(_)) => MerklePath::Left(Fp::from(7u64)),
                (0, MerklePath::Right(_)) => MerklePath::Right(Fp::from(7u64)),
                (_, MerklePath::Left(h)) => MerklePath::Left(*h),
                (_, MerklePath::Right(h)) => MerklePath::Right(*h),
            })
            .collect();
        assert_ne!(
            implied_root(&account, &tampered),
            root,
            "a tampered path must not fold to the ledger root"
        );
    }
}
