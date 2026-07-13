//! Two diagnostics that pin the mesa implementation against mina-tree as ground truth,
//! each testing exactly one thing so a genesis-root mismatch has somewhere to point.
//!
//! 1. **The tree.** Feed mina-tree's own V2 leaf hashes through *our* tree and demand the
//!    same root mina-tree computes. Independent of mesa account hashing entirely.
//! 2. **The account packing.** Compare the field vectors our `ToInputs` emits against
//!    mina-tree's for the *same* account. They must agree everywhere except the one field
//!    that is the folded zkApp hash -- which is precisely what mesa changes.

use mina_curves::pasta::Fp;
use mina_signer::CompressedPubKey;
use mina_tree::{
    Account as TreeAccount, BaseLedger, Database, Mask, ToInputs as _,
};
use mina_verify::mesa::{account::MesaAccount, ledger::MesaLedger};

/// Our tree, driven by mina-tree's V2 leaves, must reproduce mina-tree's V2 root.
#[test]
fn our_tree_reproduces_mina_trees_root() {
    // Distinct *public keys* matter: AccountId is (pubkey, token), and
    // `get_or_create_account` dedupes on it -- accounts differing only in balance would
    // collapse to one in mina-tree's ledger while our tree happily hashed all of them.
    let accounts: Vec<TreeAccount> = (0..5u64)
        .map(|i| {
            let mut account = TreeAccount::empty();
            account.public_key = CompressedPubKey {
                x: Fp::from(i + 1),
                is_odd: i % 2 == 0,
            };
            account.balance = mina_tree::scan_state::currency::Balance::of_nanomina_int_exn(
                1_000_000_000 + i * 7,
            );
            account
        })
        .collect();

    // mina-tree's answer
    let mut mask = Mask::new_root(Database::create(35));
    for account in accounts.iter() {
        mask.get_or_create_account(account.id(), account.clone())
            .expect("insert");
    }
    let expected = mask.merkle_root();

    // ours, given the same leaves and the same empty account
    let leaves: Vec<Fp> = accounts.iter().map(TreeAccount::hash).collect();
    let ours = MesaLedger::from_leaves_with_empty(leaves, TreeAccount::empty().hash());

    assert_eq!(
        ours.merkle_root(),
        expected,
        "our Merkle fold disagrees with mina-tree's on identical leaves -- the tree is wrong"
    );
}

/// Our account packing must be mina-tree's, field for field, apart from the folded zkApp
/// hash (index 0) -- the only thing mesa changes.
#[test]
fn our_account_packing_matches_mina_trees() {
    let mut tree = TreeAccount::empty();
    tree.balance = mina_tree::scan_state::currency::Balance::of_nanomina_int_exn(123_456_789);
    tree.nonce = mina_tree::scan_state::currency::Nonce::from_u32(7);

    // the same account, in our model
    let mut mesa = MesaAccount::empty();
    mesa.public_key = tree.public_key.clone();
    mesa.token_id = tree.token_id.clone();
    mesa.token_symbol = tree.token_symbol.clone();
    mesa.balance = tree.balance;
    mesa.nonce = tree.nonce;
    mesa.receipt_chain_hash = tree.receipt_chain_hash.clone();
    mesa.delegate = tree.delegate.clone();
    mesa.voting_for = tree.voting_for.clone();
    mesa.timing = tree.timing.clone();
    mesa.permissions = tree.permissions.clone();

    let tree_fields = tree.to_inputs_owned().to_fields();
    let mesa_fields = mesa.to_inputs_owned().to_fields();

    assert_eq!(
        tree_fields.len(),
        mesa_fields.len(),
        "packed field count differs -- we are appending a different shape than mina-tree"
    );

    // index 0 is the folded zkApp hash: mesa's default zkApp is 32-wide, so it *must*
    // differ. Everything after it is the plain account, and must be identical.
    assert_ne!(
        tree_fields[0], mesa_fields[0],
        "the default zkApp hash is identical -- the 32-wide state is not being hashed"
    );
    assert_eq!(
        &tree_fields[1..],
        &mesa_fields[1..],
        "our account packing diverges from mina-tree's outside the zkApp field"
    );
}

/// The MINA token is field element 1. The mesa dump spells it base58; Berkeley spelled it
/// "1". If our base58 decode does not land on `TokenId::default()`, *every* account in the
/// ledger hashes wrong -- which looks exactly like a broken root.
#[test]
fn mina_token_base58_decodes_to_the_default_token() {
    use mina_p2p_messages::v2::TokenIdKeyHash;
    use mina_tree::TokenId;
    use std::str::FromStr;

    const MINA_TOKEN_B58: &str = "wSHV2S4qX9jFsLjQo8r1BsMLH2ZRKsZx6EJd1sbozGPieEC4Jf";

    let decoded = TokenId::from(TokenIdKeyHash::from_str(MINA_TOKEN_B58).expect("base58"));

    assert_eq!(
        decoded,
        TokenId::default(),
        "base58 MINA token id does not decode to the default token"
    );
}
