//! The empty-ledger root depends on nothing but the empty account, and the mesa-mut daemon
//! says it is `jwkaDMeS...`. Ours disagrees, so our empty account is wrong. Enumerate the
//! plausible variants and see which one the protocol actually uses.

use mina_curves::pasta::Fp;
use mina_p2p_messages::v2::{LedgerHash, MinaBaseLedgerHash0StableV1};
use mina_tree::{
    scan_state::currency::TxnVersion, AuthRequired, Permissions, SetVerificationKey, ToInputs,
};
use mina_verify::mesa::{
    account::{AccountOf, ZkAppAccountOf},
    ledger::{hash_node, MESA_LEDGER_DEPTH},
};

/// what the mesa-mut daemon computes for `[]`
const ORACLE_EMPTY_LEDGER: &str = "jwkaDMeSMeL94hwJg6EYtSRD6gxB1BHpXnGKnZ1VW6jthKzgcef";
/// what the *devnet* daemon computes for `[]` -- a second network to pin against
const ORACLE_DEVNET_EMPTY: &str = "jxvN5DVDHPQow7qV8qMYu5JViwcLwYaM885xoov9DdSeC6oGMfc";

fn empty_root(empty_leaf: Fp) -> String {
    let mut h = empty_leaf;
    for height in 0..MESA_LEDGER_DEPTH {
        h = hash_node(height, h, h);
    }
    let hash: LedgerHash = MinaBaseLedgerHash0StableV1(h.into()).into();
    hash.to_string()
}

fn user_default_with(txn_version: u32) -> Permissions<AuthRequired> {
    use AuthRequired::*;
    Permissions {
        edit_state: Signature,
        access: None,
        send: Signature,
        receive: None,
        set_delegate: Signature,
        set_permissions: Signature,
        set_verification_key: SetVerificationKey {
            auth: Signature,
            txn_version: TxnVersion::from_u32(txn_version),
        },
        set_zkapp_uri: Signature,
        edit_action_state: Signature,
        set_token_symbol: Signature,
        increment_nonce: Signature,
        set_voting_for: Signature,
        set_timing: Signature,
    }
}

/// the empty account's hash, for a given zkApp width and permissions txn_version
fn empty_leaf<const N: usize>(txn_version: u32) -> Fp {
    let mut account = AccountOf::<N>::empty_with_txn_version(txn_version);
    account.permissions = user_default_with(txn_version);
    account.hash()
}

#[test]
fn solve_for_the_empty_account() {
    println!("oracle (mesa-mut daemon, empty ledger): {ORACLE_EMPTY_LEDGER}\n");
    println!("zkapp_width  txn_version  root");

    let mut hit = false;

    for txn_version in 0..=8u32 {
        for (width, leaf) in [
            (8usize, empty_leaf::<8>(txn_version)),
            (32usize, empty_leaf::<32>(txn_version)),
        ] {
            let root = empty_root(leaf);
            let marker = if root == ORACLE_EMPTY_LEDGER {
                hit = true;
                "  <<<<< MESA-MUT"
            } else if root == ORACLE_DEVNET_EMPTY {
                "  <<<<< DEVNET"
            } else {
                ""
            };
            println!("{width:>11}  {txn_version:>11}  {root}{marker}");
        }
    }

    // also: what the default zkApp digest would have to be, per width
    println!("\ndefault zkapp digest (ours):");
    println!("  width  8: {}", ZkAppAccountOf::<8>::default().hash());
    println!("  width 32: {}", ZkAppAccountOf::<32>::default().hash());

    assert!(hit, "none of the enumerated empty accounts reproduces the daemon's empty ledger");
}
