//! Mesa (protocol transaction version 3) ledger support.
//!
//! Mesa widened the zkApp application state from 8 field elements to 32, which puts every
//! mesa account out of reach of `mina-tree`'s `ZkAppAccount` (`[Fp; 8]`) -- and with it
//! every mesa ledger root and account inclusion proof.
//!
//! This adds the mesa account model and its ledger, hashed with mina-tree's *public*
//! primitives (the same `ToInputs` packing, the same Poseidon parameters, the same node
//! hashing). Nothing in mina-tree is forked or modified.
//!
//! The acceptance test is exact and unforgiving: the mesa genesis state dump must hash to
//! the ledger hash the protocol published,
//! `jxicjVogngTDjJh5EEsTUrvBxa3R4fhepqrAeexiRVMogJGqHdT`. There is no partial credit --
//! see `tests/mesa_genesis_root.rs`.

pub mod account;
pub mod json;
pub mod ledger;

pub use account::{
    default_zkapp_hash_of, AccountOf, V2Account, V2ZkAppAccount, ZkAppAccountOf,
    V2_ZKAPP_STATE_SIZE,
    default_mesa_zkapp_hash, MesaAccount, MesaZkAppAccount, MESA_ZKAPP_STATE_SIZE,
};
pub use ledger::{
    empty_hash_at_height, hash_node, implied_root, verify_account_inclusion, MesaLedger,
    MESA_LEDGER_DEPTH,
};
