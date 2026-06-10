//! Load a blockchain verifier index from JSON for *any* network.
//!
//! `mina-tree` only embeds devnet (current) and mainnet (stale-format) verifier
//! indexes, and the finalization that turns a parsed index into a usable one
//! (`make_verifier_index`) is private. This module re-implements that finalization
//! with public kimchi APIs, so a caller can supply the index JSON for any network —
//! mesa-mut, a future hardfork, or a regenerated mainnet — via
//! [`crate::Verifier::with_index_json`].

use std::sync::Arc;

use kimchi::circuits::constraints::FeatureFlags;
use kimchi::circuits::expr::Linearization;
use kimchi::circuits::lookup::lookups::{LookupFeatures, LookupPatterns};
use kimchi::circuits::polynomials::permutation::{permutation_vanishing_polynomial, zk_w};
use kimchi::linearization::expr_linearization;
use mina_curves::pasta::Fq;
use mina_tree::proofs::transaction::endos;
use mina_tree::proofs::VerifierIndex;
use once_cell::sync::OnceCell;
use poly_commitment::ipa::SRS;
use poly_commitment::SRS as _;

/// Parse and finalize a blockchain verifier index from its JSON form (the
/// `*_blockchain_verifier_index.json` format with hex-string fields).
pub fn verifier_index_from_json(json: &str) -> Result<VerifierIndex<Fq>, serde_json::Error> {
    let index: VerifierIndex<Fq> = serde_json::from_str(json)?;
    Ok(finalize(index))
}

/// Mirror of mina-tree's private `make_verifier_index`: attach the SRS + lagrange
/// basis, endo, linearization, and permutation polynomials to a parsed index.
fn finalize(index: VerifierIndex<Fq>) -> VerifierIndex<Fq> {
    let domain = index.domain;
    let max_poly_size: usize = index.max_poly_size;
    let (endo, _) = endos::<Fq>();

    let feature_flags = FeatureFlags {
        range_check0: false,
        range_check1: false,
        foreign_field_add: false,
        foreign_field_mul: false,
        xor: false,
        rot: false,
        lookup_features: LookupFeatures {
            patterns: LookupPatterns {
                xor: false,
                lookup: false,
                range_check: false,
                foreign_field_mul: false,
            },
            joint_lookup_used: false,
            uses_runtime_tables: false,
        },
    };

    let (mut linearization, powers_of_alpha) = expr_linearization(Some(&feature_flags), true);
    let linearization = Linearization {
        constant_term: linearization.constant_term,
        index_terms: {
            linearization
                .index_terms
                .sort_by_key(|&(columns, _)| columns);
            linearization.index_terms
        },
    };

    let srs = {
        let srs = SRS::create(max_poly_size);
        srs.get_lagrange_basis(domain);
        Arc::new(srs)
    };

    let permutation_vanishing_polynomial_m =
        permutation_vanishing_polynomial(domain, index.zk_rows);
    let w = zk_w(domain, index.zk_rows);

    VerifierIndex::<Fq> {
        srs,
        permutation_vanishing_polynomial_m: OnceCell::from(permutation_vanishing_polynomial_m),
        w: OnceCell::from(w),
        endo,
        linearization,
        powers_of_alpha,
        ..index
    }
}
