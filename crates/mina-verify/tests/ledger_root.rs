//! Hash a ledger someone else produced and check it against a root the protocol committed
//! to.
//!
//! This is the acceptance test for an indexer. The other tests here build the ledger
//! *themselves*, from the genesis dump and the blocks, and so can only prove that our
//! hashing is right. This one takes a ledger an indexer emitted -- its accounts, in leaf
//! order -- and asks the only question that matters to someone reading a balance off that
//! indexer: **does what it holds hash to the root the block's proof covers?**
//!
//! A mismatch means the indexer's ledger is not the protocol's, and every balance it
//! serves is suspect. There is no partial credit: one wrong field in one account is a
//! different root.
//!
//! ```sh
//! NETWORK=devnet \                      # or mesa
//! LEDGER=/path/to/indexer-ledger.json \ # {"ledger": {"accounts": [...]}}, in leaf order
//! EXPECTED=jwbzhJzXhPVVYxYPVvDCcvRNRmoRxGYZR7641Wiz7PtwaLboRaP \
//!   cargo test --release -p mina-verify --test ledger_root -- --ignored --nocapture
//! ```

use mina_p2p_messages::v2::{LedgerHash, MinaBaseLedgerHash0StableV1};
use mina_verify::mesa::{
    json::{Account as JsonAccount, Ledger as JsonLedger},
    AccountOf, LedgerParams, MesaLedger, DEVNET, MESA,
};
use std::time::Instant;

#[derive(serde::Deserialize)]
struct LedgerFile {
    ledger: JsonLedger,
}

fn to_base58(root: mina_curves::pasta::Fp) -> String {
    let hash: LedgerHash = MinaBaseLedgerHash0StableV1(root.into()).into();
    hash.to_string()
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("set {key}"))
}

/// `N` is the network's zkApp app-state width -- 32 on mesa, 8 on devnet.
fn root_of<const N: usize>(json: &[JsonAccount], params: LedgerParams) -> String {
    let accounts: Vec<AccountOf<N>> = json
        .iter()
        .enumerate()
        .map(|(i, a)| {
            a.to_account_of::<N>(params)
                .unwrap_or_else(|e| panic!("account at leaf {i}: {e}"))
        })
        .collect();

    let zkapps = accounts.iter().filter(|a| a.zkapp.is_some()).count();
    println!("accounts    : {} ({zkapps} zkApps)", accounts.len());

    let t = Instant::now();
    let root = to_base58(MesaLedger::new(&accounts, params).merkle_root());
    println!("hashed      : {:.2?}", t.elapsed());

    root
}

#[test]
#[ignore = "needs a ledger dump; set NETWORK, LEDGER and EXPECTED"]
fn a_ledger_hashes_to_the_root_the_protocol_committed_to() {
    let network = std::env::var("NETWORK").unwrap_or_else(|_| "mesa".into());
    let path = env("LEDGER");
    let expected = env("EXPECTED");

    let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("opening {path}: {e}"));
    let ledger: LedgerFile = serde_json::from_reader(std::io::BufReader::new(file))
        .unwrap_or_else(|e| panic!("parsing {path}: {e}"));
    let json = ledger
        .ledger
        .accounts
        .expect("the file has no /ledger/accounts");

    println!("network     : {network}");

    let got = match network.as_str() {
        "mesa" => root_of::<32>(&json, MESA),
        "devnet" => root_of::<8>(&json, DEVNET),
        other => panic!("unknown network {other:?} -- expected \"mesa\" or \"devnet\""),
    };

    println!("computed    : {got}");
    println!("expected    : {expected}");

    assert_eq!(
        got, expected,
        "this ledger does not hash to the root the protocol committed to -- \
         the accounts it holds are not the protocol's"
    );
}
