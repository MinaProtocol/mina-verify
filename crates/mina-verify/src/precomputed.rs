//! Verify **precomputed blocks** — the JSON block format daemons publish (and indexers
//! ingest from GCS). A precomputed block carries the same `protocol_state` +
//! `protocol_state_proof` as a network block, so it can be verified exactly like one:
//! decode the JSON into a block header, then [`Verifier::verify_header`].
//!
//! This is the trustless gate for an indexer: verify each precomputed block before
//! ingesting it, so every row the indexer serves is proof-backed.

use std::sync::Arc;

use base64::Engine;
use binprot::BinProtRead;
use mina_p2p_messages::list::List;
use mina_p2p_messages::number::UInt64;
use mina_p2p_messages::v2::{
    MinaBaseProofStableV2, MinaBlockHeaderStableV2, MinaStateProtocolStateValueStableV2,
    ProtocolVersionStableV2,
};

use crate::{BlockHeader, Verifier, VerifierError};

/// Decode a precomputed-block JSON (the `{ "version", "data": { … } }` form, or a bare
/// block object) into a verifiable [`BlockHeader`]. Only the fields that participate in
/// proof verification (`protocol_state`, `protocol_state_proof`) plus the header's other
/// required fields are read; the staged-ledger diff / accounts lists are ignored.
pub fn header_from_precomputed(json: &str) -> Result<BlockHeader, VerifierError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(invalid)?;
    let data = v.get("data").unwrap_or(&v);

    let field = |name: &str| -> Result<serde_json::Value, VerifierError> {
        data.get(name).cloned().ok_or_else(|| {
            VerifierError::InvalidIndexJson(format!("precomputed block missing `{name}`"))
        })
    };

    let protocol_state: MinaStateProtocolStateValueStableV2 =
        serde_json::from_value(field("protocol_state")?).map_err(invalid)?;

    // The proof is a URL-safe-base64 string of the binprot proof.
    let proof_b64: String =
        serde_json::from_value(field("protocol_state_proof")?).map_err(invalid)?;
    let proof_bytes = base64::engine::general_purpose::URL_SAFE
        .decode(proof_b64.as_bytes())
        .map_err(invalid)?;
    let proof = MinaBaseProofStableV2::binprot_read(&mut &proof_bytes[..]).map_err(invalid)?;

    // delta_block_chain_proof + current_protocol_version don't participate in proof
    // verification, so fill placeholders rather than parse their base58check forms.
    let zero = || UInt64::from(0u64);
    Ok(MinaBlockHeaderStableV2 {
        delta_block_chain_proof: (protocol_state.previous_state_hash.clone(), List::new()),
        current_protocol_version: ProtocolVersionStableV2 {
            transaction: zero(),
            network: zero(),
            patch: zero(),
        },
        protocol_state,
        protocol_state_proof: Arc::new(proof),
        proposed_protocol_version_opt: None,
    })
}

fn invalid(e: impl std::fmt::Display) -> VerifierError {
    VerifierError::InvalidIndexJson(e.to_string())
}

impl Verifier {
    /// Decode and verify a precomputed block's proof on this network's VK.
    pub fn verify_precomputed_block(&self, json: &str) -> Result<bool, VerifierError> {
        Ok(self.verify_header(&header_from_precomputed(json)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real devnet precomputed block ({version,data} envelope; not strictly UTF-8). These
    // tests exercise only the JSON/decode path — no SNARK proof runs — so they're fast.
    const FIXTURE_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/devnet-528700.json"
    );

    fn fixture_json() -> String {
        let bytes = std::fs::read(FIXTURE_PATH).expect("read fixture block");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn height(h: &BlockHeader) -> u32 {
        h.protocol_state
            .body
            .consensus_state
            .blockchain_length
            .as_u32()
    }

    #[test]
    fn decodes_a_real_precomputed_block() {
        let header = header_from_precomputed(&fixture_json()).expect("decode");
        assert_eq!(height(&header), 528700);
    }

    #[test]
    fn accepts_a_bare_block_without_the_data_envelope() {
        // header_from_precomputed unwraps `data` if present, else treats the value as the
        // block. Feed the inner object directly to cover the bare path.
        let v: serde_json::Value = serde_json::from_str(&fixture_json()).unwrap();
        let bare = v.get("data").expect("data envelope").to_string();
        let header = header_from_precomputed(&bare).expect("decode bare");
        assert_eq!(height(&header), 528700);
    }

    #[test]
    fn rejects_non_json() {
        let err = header_from_precomputed("this is not json").unwrap_err();
        assert!(matches!(err, VerifierError::InvalidIndexJson(_)), "{err:?}");
    }

    #[test]
    fn rejects_missing_protocol_state() {
        let err = header_from_precomputed("{}").unwrap_err();
        match err {
            VerifierError::InvalidIndexJson(m) => assert!(m.contains("protocol_state"), "{m}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_proof() {
        let mut v: serde_json::Value = serde_json::from_str(&fixture_json()).unwrap();
        v["data"]
            .as_object_mut()
            .unwrap()
            .remove("protocol_state_proof");
        let err = header_from_precomputed(&v.to_string()).unwrap_err();
        match err {
            VerifierError::InvalidIndexJson(m) => {
                assert!(m.contains("protocol_state_proof"), "{m}")
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn rejects_non_base64_proof() {
        let mut v: serde_json::Value = serde_json::from_str(&fixture_json()).unwrap();
        v["data"]["protocol_state_proof"] = serde_json::json!("!!! not base64 !!!");
        let err = header_from_precomputed(&v.to_string()).unwrap_err();
        assert!(matches!(err, VerifierError::InvalidIndexJson(_)), "{err:?}");
    }
}
