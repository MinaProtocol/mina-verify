//! Pin devnet's ledger parameters against the protocol's published root.
//!
//! `mesa/account.rs` deliberately leaves devnet's `LedgerParams` unpinned: the only devnet
//! image to hand was a generic build, so its constants were never confirmed against
//! anything. This confirms them the only way that admits no ambiguity -- by hashing
//! devnet's genesis state dump and reproducing the root the chain itself carries in every
//! block's `blockchain_state.genesis_ledger_hash`.
//!
//! devnet's app state is 8 fields wide (mesa's is 32). The `txn_version` enters the hash
//! only through the *empty* account -- every stated account overrides its permissions --
//! so it is pinned by sweeping it and seeing which value reproduces the root.
//!
//! ```sh
//! DEVNET_GENESIS_LEDGER=/path/to/devnet-genesis.json \
//!   cargo test --release -p mina-verify --test devnet_genesis_root -- --ignored --nocapture
//! ```

use mina_p2p_messages::v2::{LedgerHash, MinaBaseLedgerHash0StableV1};
use mina_verify::mesa::{
    json::{Account as JsonAccount, Ledger as JsonLedger},
    LedgerParams, MesaLedger, V2Account, DEVNET,
};
use std::time::Instant;

/// devnet's genesis ledger hash: what every devnet block carries in
/// `blockchain_state.genesis_ledger_hash`, and what the indexer pins as
/// `DEVNET_GENESIS_LEDGER_HASH`.
const DEVNET_GENESIS_LEDGER_HASH: &str = "jwX3YJhLR5F3eByADvfurX5u7DT7Utiv54uixYts6HLrR6CETug";

#[derive(serde::Deserialize)]
struct GenesisFile {
    ledger: JsonLedger,
}

fn to_base58(root: mina_curves::pasta::Fp) -> String {
    let hash: LedgerHash = MinaBaseLedgerHash0StableV1(root.into()).into();
    hash.to_string()
}

#[test]
#[ignore = "needs the devnet genesis state dump; set DEVNET_GENESIS_LEDGER"]
fn devnet_genesis_ledger_hashes_to_the_protocol_root() {
    let path = std::env::var("DEVNET_GENESIS_LEDGER")
        .expect("set DEVNET_GENESIS_LEDGER to the devnet genesis state dump");

    let t = Instant::now();
    let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("opening {path}: {e}"));
    let genesis: GenesisFile = serde_json::from_reader(std::io::BufReader::new(file))
        .unwrap_or_else(|e| panic!("parsing {path} as a genesis state dump: {e}"));
    let json: Vec<JsonAccount> = genesis
        .ledger
        .accounts
        .expect("genesis file has no /ledger/accounts");
    println!("parsed  : {} accounts in {:.2?}", json.len(), t.elapsed());

    // the txn_version reaches the root only through the empty account, so sweep it
    let mut pinned = None;

    for txn_version in 0..=8u32 {
        let params = LedgerParams {
            zkapp_state_size: 8,
            txn_version,
            depth: 35,
        };

        let accounts: Vec<V2Account> = json
            .iter()
            .enumerate()
            .map(|(i, a)| {
                a.to_account_of::<8>(params)
                    .unwrap_or_else(|e| panic!("account #{i}: {e}"))
            })
            .collect();

        let zkapps = accounts.iter().filter(|a| a.zkapp.is_some()).count();
        let got = to_base58(MesaLedger::new(&accounts, params).merkle_root());
        let hit = got == DEVNET_GENESIS_LEDGER_HASH;

        println!(
            "txn_version {txn_version}: {got} {}",
            if hit { "  <-- MATCH" } else { "" }
        );

        if hit {
            pinned = Some((params, zkapps));
            break;
        }
    }

    let (params, zkapps) = pinned.unwrap_or_else(|| {
        panic!(
            "no txn_version in 0..=8 reproduces devnet's genesis root {DEVNET_GENESIS_LEDGER_HASH} \
             at width 8 / depth 35 -- devnet's parameters are something else"
        )
    });

    println!(
        "\ndevnet pinned: zkapp_state_size {}, txn_version {}, depth {} ({zkapps} zkApp accounts)",
        params.zkapp_state_size, params.txn_version, params.depth
    );

    assert!(
        zkapps > 0,
        "no zkApp accounts parsed -- devnet is mostly zkApps, so something is wrong"
    );

    // ...and that is what `DEVNET` claims
    assert_eq!(params.zkapp_state_size, DEVNET.zkapp_state_size);
    assert_eq!(params.txn_version, DEVNET.txn_version);
    assert_eq!(params.depth, DEVNET.depth);
}
