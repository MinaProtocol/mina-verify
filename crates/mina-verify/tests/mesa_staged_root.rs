//! Does a block's `staged_ledger_hash` equal the merkle root of the accounts the ledger
//! holds at that block?
//!
//! `mesa_genesis_root` and `devnet_genesis_root` prove the account model against each
//! network's *genesis* ledger. This proves the next link, and it is the one an indexer
//! needs: replay each block's `accounts_accessed` (every touched account, with its ledger
//! index and full post-state) onto the genesis ledger, and the tree must hash to the
//! `staged_ledger_hash` that block committed to. That hash is covered by the block's
//! Pickles proof, so reproducing it is what turns a served balance into a *verifiable* one
//! instead of a trusted one.
//!
//! Two facts about ledger indices fall out of this and are asserted below: genesis
//! accounts sit at their **file order**, and accounts created after genesis are
//! **appended contiguously** in creation order.
//!
//! The state dumps are large and the blocks larger, so neither is committed. Point the
//! test at them; blocks are replayed in the order given, and each must be the child of the
//! last:
//!
//! ```sh
//! NETWORK=devnet \                       # or mesa (the default)
//! GENESIS_LEDGER=/path/to/devnet-genesis.json \
//! BLOCKS=/path/to/devnet-527923-3N….json,/path/to/devnet-527924-3N….json \
//!   cargo test --release -p mina-verify --test mesa_staged_root -- --ignored --nocapture
//! ```

use mina_p2p_messages::v2::{LedgerHash, MinaBaseLedgerHash0StableV1};
use mina_verify::mesa::{
    accounts_accessed,
    json::{Account as JsonAccount, Ledger as JsonLedger},
    staged_ledger_hash, AccountOf, LedgerParams, MesaLedger, DEVNET, MESA,
};
use std::time::Instant;

/// What each network's blocks carry in `blockchain_state.genesis_ledger_hash`.
const MESA_GENESIS_LEDGER_HASH: &str = "jxicjVogngTDjJh5EEsTUrvBxa3R4fhepqrAeexiRVMogJGqHdT";
const DEVNET_GENESIS_LEDGER_HASH: &str = "jwX3YJhLR5F3eByADvfurX5u7DT7Utiv54uixYts6HLrR6CETug";

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

/// Precomputed blocks are not valid UTF-8: the daemon writes
/// `ledger_proof_statement.sok_digest` as a raw byte string. Nothing we read lives in
/// there, so decode leniently rather than refuse the block.
fn read_block(path: &str) -> serde_json::Value {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    serde_json::from_str(&String::from_utf8_lossy(&bytes))
        .unwrap_or_else(|e| panic!("parsing {path}: {e}"))
}

/// `N` is the network's zkApp app-state width: 32 on mesa, 8 on devnet. It is a const
/// generic, so the two networks need two instantiations of the same code.
fn replay<const N: usize>(
    json: &[JsonAccount],
    params: LedgerParams,
    genesis_root: &str,
    blocks: &[&str],
) {
    let to_account = |account: &JsonAccount, what: &str| -> AccountOf<N> {
        account
            .to_account_of::<N>(params)
            .unwrap_or_else(|e| panic!("{what}: {e}"))
    };

    let mut accounts: Vec<AccountOf<N>> = json
        .iter()
        .enumerate()
        .map(|(i, a)| to_account(a, &format!("genesis account #{i}")))
        .collect();
    let genesis_len = accounts.len();

    // control: genesis accounts in file order must already be the published root,
    // otherwise nothing below means anything
    let t = Instant::now();
    let got = to_base58(MesaLedger::new(&accounts, params).merkle_root());
    println!("genesis root: {got} ({:.2?})", t.elapsed());
    assert_eq!(
        got, genesis_root,
        "the genesis ledger does not hash to the protocol root -- fix that first"
    );

    let (mut updated, mut created) = (0usize, 0usize);

    for path in blocks {
        let block = read_block(path);
        let expected = staged_ledger_hash(&block)
            .unwrap_or_else(|| panic!("{path} has no staged_ledger_hash"))
            .to_owned();

        let mut accessed = accounts_accessed(&block)
            .unwrap_or_else(|e| panic!("reading accounts_accessed of {path}: {e}"));

        // created accounts take the next free index, so apply in index order
        accessed.sort_by_key(|(index, _)| *index);

        for (index, account) in &accessed {
            let account = to_account(account, &format!("{path}: account at index {index}"));

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

        let got = to_base58(MesaLedger::new(&accounts, params).merkle_root());
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

#[test]
#[ignore = "needs a genesis state dump and blocks; set GENESIS_LEDGER and BLOCKS (and NETWORK)"]
fn a_blocks_staged_ledger_hash_is_the_root_of_genesis_plus_accounts_accessed() {
    let network = std::env::var("NETWORK").unwrap_or_else(|_| "mesa".into());
    let genesis_path = env("GENESIS_LEDGER");
    let blocks = env("BLOCKS");
    let blocks: Vec<&str> = blocks.split(',').map(str::trim).collect();

    let t = Instant::now();
    let file = std::fs::File::open(&genesis_path).expect("opening the genesis dump");
    let genesis: GenesisFile = serde_json::from_reader(std::io::BufReader::new(file))
        .expect("parsing the genesis state dump");
    let json = genesis
        .ledger
        .accounts
        .expect("genesis file has no /ledger/accounts");
    println!(
        "network     : {network}\nparsed      : {} accounts in {:.2?}",
        json.len(),
        t.elapsed()
    );

    match network.as_str() {
        "mesa" => replay::<32>(&json, MESA, MESA_GENESIS_LEDGER_HASH, &blocks),
        "devnet" => replay::<8>(&json, DEVNET, DEVNET_GENESIS_LEDGER_HASH, &blocks),
        other => panic!("unknown network {other:?} -- expected \"mesa\" or \"devnet\""),
    }
}
