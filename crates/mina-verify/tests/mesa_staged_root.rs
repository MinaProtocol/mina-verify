//! Does a block's `staged_ledger_hash` equal the merkle root of the accounts the ledger
//! holds at that block?
//!
//! `mesa_genesis_root` proved the account model against the *genesis* ledger. This proves
//! the next link, and it is the one an indexer needs: replay each block's
//! `accounts_accessed` (every touched account, with its ledger index and full post-state)
//! onto the genesis ledger, and the tree must hash to the `staged_ledger_hash` that block
//! committed to. That hash is covered by the block's Pickles proof, so reproducing it is
//! what turns a served balance into a *verifiable* one instead of a trusted one.
//!
//! Two facts about mesa ledger indices fall out of this and are asserted below: genesis
//! accounts sit at their **file order**, and accounts created after genesis are
//! **appended contiguously** in creation order.
//!
//! The state dump is ~900MB and the blocks are ~4GB, so neither is committed. Point the
//! test at them; blocks are replayed in the order given, and each must be the child of the
//! last:
//!
//! ```sh
//! MESA_GENESIS_LEDGER=/path/to/mesa-genesis.json \
//! MESA_BLOCKS=/path/to/mesa-297736-3NK6….json,/path/to/mesa-297737-3NLk….json \
//!   cargo test --release -p mina-verify --test mesa_staged_root -- --ignored --nocapture
//! ```

use mina_p2p_messages::v2::{LedgerHash, MinaBaseLedgerHash0StableV1};
use mina_verify::mesa::{
    accounts_accessed,
    json::{Account as JsonAccount, Ledger as JsonLedger},
    staged_ledger_hash, MesaAccount, MesaLedger, MESA,
};
use std::time::Instant;

/// The mesa genesis ledger hash, as published by the protocol. Every mesa-mut block
/// carries it in `blockchain_state.genesis_ledger_hash`.
const MESA_GENESIS_LEDGER_HASH: &str = "jxicjVogngTDjJh5EEsTUrvBxa3R4fhepqrAeexiRVMogJGqHdT";

#[derive(serde::Deserialize)]
struct GenesisFile {
    ledger: JsonLedger,
}

fn to_base58(root: mina_curves::pasta::Fp) -> String {
    let hash: LedgerHash = MinaBaseLedgerHash0StableV1(root.into()).into();
    hash.to_string()
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("set {key}"))
}

fn to_mesa(account: &JsonAccount, what: &str) -> MesaAccount {
    account
        .to_account_of::<32>(MESA)
        .unwrap_or_else(|e| panic!("{what}: {e}"))
}

/// Precomputed blocks are not valid UTF-8: the daemon writes
/// `ledger_proof_statement.sok_digest` as a raw byte string. Nothing we read lives in
/// there, so decode leniently rather than refuse the block.
fn read_block(path: &str) -> serde_json::Value {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    serde_json::from_str(&String::from_utf8_lossy(&bytes))
        .unwrap_or_else(|e| panic!("parsing {path}: {e}"))
}

#[test]
#[ignore = "needs the ~900MB mesa genesis state dump and mesa blocks; set MESA_GENESIS_LEDGER and MESA_BLOCKS"]
fn a_blocks_staged_ledger_hash_is_the_root_of_genesis_plus_accounts_accessed() {
    let genesis_path = env("MESA_GENESIS_LEDGER");
    let blocks = env("MESA_BLOCKS");
    let blocks: Vec<&str> = blocks.split(',').map(str::trim).collect();

    // the ledger as of the genesis (fork) block
    let t = Instant::now();
    let file = std::fs::File::open(&genesis_path).expect("opening the genesis dump");
    let genesis: GenesisFile = serde_json::from_reader(std::io::BufReader::new(file))
        .expect("parsing the genesis state dump");
    let json = genesis
        .ledger
        .accounts
        .expect("genesis file has no /ledger/accounts");
    println!(
        "parsed      : {} accounts in {:.2?}",
        json.len(),
        t.elapsed()
    );

    let mut accounts: Vec<MesaAccount> = json
        .iter()
        .enumerate()
        .map(|(i, a)| to_mesa(a, &format!("genesis account #{i}")))
        .collect();
    let genesis_len = accounts.len();

    // control: genesis accounts in file order must already be the published root,
    // otherwise nothing below means anything
    let t = Instant::now();
    let genesis_root = to_base58(MesaLedger::new(&accounts, MESA).merkle_root());
    println!("genesis root: {genesis_root} ({:.2?})", t.elapsed());
    assert_eq!(
        genesis_root, MESA_GENESIS_LEDGER_HASH,
        "the genesis ledger does not hash to the protocol root -- fix that first"
    );

    let (mut updated, mut created) = (0usize, 0usize);

    for path in &blocks {
        let block = read_block(path);
        let expected = staged_ledger_hash(&block)
            .unwrap_or_else(|| panic!("{path} has no staged_ledger_hash"))
            .to_owned();

        let mut accessed = accounts_accessed(&block)
            .unwrap_or_else(|e| panic!("reading accounts_accessed of {path}: {e}"));

        // created accounts take the next free index, so apply in index order
        accessed.sort_by_key(|(index, _)| *index);

        for (index, account) in &accessed {
            let account = to_mesa(account, &format!("{path}: account at index {index}"));

            if *index < accounts.len() {
                accounts[*index] = account;
                updated += 1;
            } else {
                assert_eq!(
                    *index,
                    accounts.len(),
                    "gap in the ledger indices at {path} -- a created account is missing, \
                     so the blocks are not a contiguous chain"
                );
                accounts.push(account);
                created += 1;
            }
        }

        let got = to_base58(MesaLedger::new(&accounts, MESA).merkle_root());
        let name = path.rsplit('/').next().unwrap_or(path);

        println!(
            "{name}\n  {} accessed, ledger now {} accounts\n  computed {got}\n  expected {expected}",
            accessed.len(),
            accounts.len(),
        );

        assert_eq!(
            got, expected,
            "{path}: staged_ledger_hash is NOT the root of the ledger its own \
             accounts_accessed implies"
        );
    }

    println!(
        "\nreplayed {} blocks: {updated} accounts updated, {created} created \
         (genesis had {genesis_len}, ledger now {})",
        blocks.len(),
        accounts.len(),
    );
}
