//! The acceptance test for mesa ledger support, and it is not a soft one.
//!
//! The protocol published the mesa genesis ledger hash. If our account model, our
//! `ToInputs` packing, our Poseidon parameters and our tree are all correct -- for every
//! one of the 277,307 accounts, including the 1,818 zkApps whose 32-wide state is the
//! whole point -- then hashing the genesis state dump reproduces it exactly. If any single
//! field of any single account is packed wrong, the root is wrong. There is no partial
//! credit and no ambiguity.
//!
//! The state dump is ~900 MB, so it is not committed. Point the test at one:
//!
//! ```sh
//! MESA_GENESIS_LEDGER=/path/to/mesa-genesis.json \
//!   cargo test -p mina-verify --test mesa_genesis_root -- --ignored --nocapture
//! ```

use mina_p2p_messages::v2::{LedgerHash, MinaBaseLedgerHash0StableV1};
use mina_verify::mesa::{
    implied_root,
    json::{Account as JsonAccount, Ledger as JsonLedger},
    MesaAccount, MesaLedger,
};
use std::time::Instant;

/// The mesa genesis ledger hash, as published by the protocol.
const MESA_GENESIS_LEDGER_HASH: &str = "jxicjVogngTDjJh5EEsTUrvBxa3R4fhepqrAeexiRVMogJGqHdT";

#[derive(serde::Deserialize)]
struct GenesisFile {
    ledger: JsonLedger,
}

fn load_accounts(path: &str) -> Vec<JsonAccount> {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("opening {path}: {e}"));
    let genesis: GenesisFile = serde_json::from_reader(std::io::BufReader::new(file))
        .unwrap_or_else(|e| panic!("parsing {path} as a genesis state dump: {e}"));

    genesis
        .ledger
        .accounts
        .expect("genesis file has no /ledger/accounts")
}

fn to_base58(root: mina_curves::pasta::Fp) -> String {
    let hash: LedgerHash = MinaBaseLedgerHash0StableV1(root.into()).into();
    hash.to_string()
}

#[test]
#[ignore = "needs the ~900MB mesa genesis state dump; set MESA_GENESIS_LEDGER"]
fn mesa_genesis_ledger_hashes_to_the_protocol_root() {
    let path = std::env::var("MESA_GENESIS_LEDGER")
        .expect("set MESA_GENESIS_LEDGER to the mesa genesis state dump");

    let json = load_accounts(&path);
    println!("parsed      : {} accounts", json.len());

    let t = Instant::now();
    let accounts: Vec<MesaAccount> = json
        .iter()
        .enumerate()
        .map(|(i, a)| {
            a.to_account_of::<32>()
                .unwrap_or_else(|e| panic!("account #{i}: {e}"))
        })
        .collect();
    println!("converted   : {} accounts in {:.2?}", accounts.len(), t.elapsed());

    let zkapps = accounts.iter().filter(|a| a.zkapp.is_some()).count();
    println!("zkapps      : {zkapps} (32-wide app_state)");
    assert!(zkapps > 0, "no zkApp accounts -- the mesa delta is untested");

    let t = Instant::now();
    let ledger = MesaLedger::new(&accounts);
    let root = ledger.merkle_root();
    println!("merkle_root : {:.2?}", t.elapsed());

    let got = to_base58(root);
    println!("computed    : {got}");
    println!("expected    : {MESA_GENESIS_LEDGER_HASH}");

    assert_eq!(
        got, MESA_GENESIS_LEDGER_HASH,
        "mesa genesis ledger root mismatch: the account model or its hashing is wrong"
    );

    // ...and every account must prove its own inclusion against that root
    for index in [0usize, 1, 7, accounts.len() / 2, accounts.len() - 1] {
        let path = ledger.merkle_path(index);

        assert_eq!(
            implied_root(&accounts[index], &path),
            root,
            "account #{index} does not prove inclusion in the root it belongs to"
        );
    }

    // a zkApp account too -- the 32-wide state is exactly what could break the fold
    let zkapp_index = accounts
        .iter()
        .position(|a| a.zkapp.is_some())
        .expect("a zkApp account");
    assert_eq!(
        implied_root(&accounts[zkapp_index], &ledger.merkle_path(zkapp_index)),
        root,
        "zkApp account #{zkapp_index} does not prove inclusion"
    );

    // and a forged account must NOT verify
    let mut forged = accounts[0].clone();
    forged.balance = mina_tree::scan_state::currency::Balance::of_nanomina_int_exn(999_999_999);
    assert_ne!(
        implied_root(&forged, &ledger.merkle_path(0)),
        root,
        "a tampered account still folded to the real root -- the proof is worthless"
    );
}
