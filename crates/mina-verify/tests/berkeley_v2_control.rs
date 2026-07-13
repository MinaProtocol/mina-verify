//! The control.
//!
//! The mesa genesis root does not match, but the tree and the account packing are both
//! proven correct against mina-tree (`tests/mesa_isolate.rs`). So either the *parser* is
//! wrong, or mesa's account model differs from V2 by more than the state width. Only one
//! experiment separates those.
//!
//! Run the **same** parser, the **same** account model and the **same** tree over a
//! *V2* ledger -- width 8 -- and demand the V2 genesis root the protocol published. If that
//! passes, the pipeline is sound end to end and mesa is genuinely different in some further
//! way. If it fails, the bug is ours and it is in the shared path.
//!
//! ```sh
//! BERKELEY_GENESIS_LEDGER=/path/to/hardfork.json \
//!   cargo test --release -p mina-verify --test berkeley_v2_control -- --ignored --nocapture
//! ```

use mina_curves::pasta::Fp;
use mina_p2p_messages::v2::{LedgerHash, MinaBaseLedgerHash0StableV1};
use mina_verify::mesa::{
    account::{V2Account, V2_ZKAPP_STATE_SIZE},
    json::{Account as JsonAccount, Ledger as JsonLedger},
    MesaLedger,
};
use std::time::Instant;

/// The Berkeley (mainnet hardfork) genesis ledger hash, as published by the protocol.
const BERKELEY_GENESIS_LEDGER_HASH: &str = "jwNw4qb6tnNhpQNxiMLem9WumxZTwmbSx3fYXW4FP3hZRkoQJSE";

#[derive(serde::Deserialize)]
struct GenesisFile {
    ledger: JsonLedger,
}

fn to_base58(root: Fp) -> String {
    let hash: LedgerHash = MinaBaseLedgerHash0StableV1(root.into()).into();
    hash.to_string()
}

#[test]
#[ignore = "needs the Berkeley genesis state dump; set BERKELEY_GENESIS_LEDGER"]
fn v2_genesis_ledger_hashes_to_the_protocol_root() {
    let path = std::env::var("BERKELEY_GENESIS_LEDGER")
        .expect("set BERKELEY_GENESIS_LEDGER to the Berkeley genesis state dump");

    let file = std::fs::File::open(&path).expect("open");
    let genesis: GenesisFile =
        serde_json::from_reader(std::io::BufReader::new(file)).expect("parse");
    let json: Vec<JsonAccount> = genesis.ledger.accounts.expect("accounts");
    println!("parsed      : {} accounts", json.len());

    let t = Instant::now();
    let accounts: Vec<V2Account> = json
        .iter()
        .enumerate()
        .map(|(i, a)| {
            a.to_account_of::<8>()
                .unwrap_or_else(|e| panic!("account #{i}: {e}"))
        })
        .collect();
    println!("converted   : {} in {:.2?}", accounts.len(), t.elapsed());
    println!(
        "zkapps      : {}",
        accounts.iter().filter(|a| a.zkapp.is_some()).count()
    );

    let t = Instant::now();
    let ledger = MesaLedger::new(&accounts);
    let got = to_base58(ledger.merkle_root());
    println!("merkle_root : {:.2?}", t.elapsed());

    println!("computed    : {got}");
    println!("expected    : {BERKELEY_GENESIS_LEDGER_HASH}");

    assert_eq!(
        got, BERKELEY_GENESIS_LEDGER_HASH,
        "our parser/account/tree cannot even reproduce the V2 root -- the bug is in the \
         shared path, not in mesa"
    );
}
