//! Prints our Merkle root for the first `MESA_LEDGER_LIMIT` accounts of a ledger file, so
//! it can be bisected against the daemon's `mina ledger hash` on the same prefix.

use mina_curves::pasta::Fp;
use mina_p2p_messages::v2::{LedgerHash, MinaBaseLedgerHash0StableV1};
use mina_verify::mesa::{json::Account as JsonAccount, MesaAccount, MesaLedger};

#[test]
#[ignore = "diagnostic"]
fn print_prefix_root() {
    let path = std::env::var("MESA_ACCOUNTS").expect("MESA_ACCOUNTS (bare accounts array)");
    let limit: usize = std::env::var("MESA_LEDGER_LIMIT")
        .expect("MESA_LEDGER_LIMIT")
        .parse()
        .unwrap();

    let file = std::fs::File::open(&path).expect("open");
    let json: Vec<JsonAccount> =
        serde_json::from_reader(std::io::BufReader::new(file)).expect("parse accounts array");

    let accounts: Vec<MesaAccount> = json
        .iter()
        .take(limit)
        .enumerate()
        .map(|(i, a)| a.to_account_of::<32>().unwrap_or_else(|e| panic!("#{i}: {e}")))
        .collect();

    let root: Fp = MesaLedger::new(&accounts).merkle_root();
    let hash: LedgerHash = MinaBaseLedgerHash0StableV1(root.into()).into();

    println!("OURS {} {}", accounts.len(), hash);
}
