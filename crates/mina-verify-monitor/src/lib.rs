//! Trustless block ingestion over pluggable **block sources**.
//!
//! The binary in this crate is a live gossip monitor; this library exposes the
//! reusable piece underneath it: [`verify_from`], which acquires a block from a
//! [`BlockSource`] (a precomputed JSON, or a live node over libp2p) and verifies its
//! proof, returning proof-backed [`mina_verify::VerifiedBlock`] facts.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use mina_verify::Verifier;
//! use mina_verify_monitor::{verify_from, BlockSource};
//!
//! let verifier = Verifier::for_network("devnet")?;
//!
//! // Start simple: a precomputed block (GCS / archive JSON).
//! let json = std::fs::read_to_string("block.json")?;
//! let facts = verify_from(&verifier, BlockSource::Precomputed(json)).await?;
//!
//! // Or verify a live node's best tip directly (the seed path).
//! let tip = verify_from(&verifier, BlockSource::node("devnet")).await?;
//! println!("tip height {} ledger {}", tip.height, tip.staged_ledger_hash);
//! # Ok(()) }
//! ```

pub mod source;
pub use source::{verify_from, BlockSource, SourceError, DEFAULT_NODE_DEADLINE};
