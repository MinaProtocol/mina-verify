//! Diagnostic: the genesis root does not match, but our tree and our account packing are
//! both proven correct against mina-tree. So an assumption is wrong. This probes one of
//! them -- that mesa's ledger depth is still 35 -- by folding past the top and printing the
//! root every depth would produce. If any of them is the published mesa genesis hash, the
//! depth was the bug.
//!
//! ```sh
//! MESA_GENESIS_LEDGER=/path/to/mesa-genesis.json \
//!   cargo test --release -p mina-verify --test mesa_depth_probe -- --ignored --nocapture
//! ```

use mina_curves::pasta::Fp;
use mina_p2p_messages::v2::{LedgerHash, MinaBaseLedgerHash0StableV1};
use mina_verify::mesa::{
    json::{Account as JsonAccount, Ledger as JsonLedger},
    ledger::hash_node,
    MesaAccount,
};

const MESA_GENESIS_LEDGER_HASH: &str = "jxicjVogngTDjJh5EEsTUrvBxa3R4fhepqrAeexiRVMogJGqHdT";

#[derive(serde::Deserialize)]
struct GenesisFile {
    ledger: JsonLedger,
}

fn to_base58(root: Fp) -> String {
    let hash: LedgerHash = MinaBaseLedgerHash0StableV1(root.into()).into();
    hash.to_string()
}

#[test]
#[ignore = "diagnostic; needs MESA_GENESIS_LEDGER"]
fn probe_ledger_depth() {
    let path = std::env::var("MESA_GENESIS_LEDGER").expect("set MESA_GENESIS_LEDGER");
    let file = std::fs::File::open(&path).expect("open");
    let genesis: GenesisFile =
        serde_json::from_reader(std::io::BufReader::new(file)).expect("parse");
    let json: Vec<JsonAccount> = genesis.ledger.accounts.expect("accounts");

    let accounts: Vec<MesaAccount> = json
        .iter()
        .map(|a| a.to_account_of::<32>(mina_verify::mesa::MESA).expect("convert"))
        .collect();
    println!("accounts: {}", accounts.len());

    // leaves, and the empty-subtree hash at each height
    let mut current: Vec<Fp> = accounts.iter().map(MesaAccount::hash).collect();
    let mut empty = MesaAccount::empty_with_txn_version(mina_verify::mesa::MESA_TXN_VERSION).hash();

    println!("\ndepth -> root");
    for height in 0..45usize {
        let mut next = Vec::with_capacity(current.len().div_ceil(2));
        for pair in current.chunks(2) {
            next.push(hash_node(height, pair[0], pair.get(1).copied().unwrap_or(empty)));
        }

        current = next;
        empty = hash_node(height, empty, empty);

        // once the level collapses to a single node, that node is the root for this depth
        if current.len() == 1 {
            let root = to_base58(current[0]);
            let hit = if root == MESA_GENESIS_LEDGER_HASH {
                "   <<<<< MATCH"
            } else {
                ""
            };
            println!("{:>5} -> {root}{hit}", height + 1);
        }
    }

    println!("\nexpected: {MESA_GENESIS_LEDGER_HASH}");
}
