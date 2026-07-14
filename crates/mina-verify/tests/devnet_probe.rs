//! Devnet's empty ledger matches no (width, txn_version) we tried. Sweep depth too --
//! the point being that these are *independent* protocol constants and must be pinned per
//! network, never inferred from one another.

use mina_curves::pasta::Fp;
use mina_p2p_messages::v2::{LedgerHash, MinaBaseLedgerHash0StableV1};
use mina_verify::mesa::{account::AccountOf, ledger::hash_node};

const ORACLE_DEVNET_EMPTY: &str = "jxvN5DVDHPQow7qV8qMYu5JViwcLwYaM885xoov9DdSeC6oGMfc";

fn empty_root_at(leaf: Fp, depth: usize) -> String {
    let mut h = leaf;
    for height in 0..depth {
        h = hash_node(height, h, h);
    }
    let hash: LedgerHash = MinaBaseLedgerHash0StableV1(h.into()).into();
    hash.to_string()
}

#[test]
fn sweep_devnet_params() {
    let mut found = vec![];

    for txn_version in 0..=8u32 {
        for (w, leaf) in [
            (8usize, AccountOf::<8>::empty_with_txn_version(txn_version).hash()),
            (32usize, AccountOf::<32>::empty_with_txn_version(txn_version).hash()),
        ] {
            for depth in 10..=35usize {
                if empty_root_at(leaf, depth) == ORACLE_DEVNET_EMPTY {
                    found.push((w, txn_version, depth));
                }
            }
        }
    }

    if found.is_empty() {
        println!("devnet: NO (width, txn_version, depth) in the swept space reproduces it");
        println!("  -> devnet differs in some further constant; it must be pinned from its");
        println!("     own daemon, not inferred. Exactly why LedgerParams is explicit.");
    } else {
        for (w, v, d) in &found {
            println!("devnet MATCH: width={w} txn_version={v} depth={d}");
        }
    }
}
