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
/// ab84160: kimchi `VerifierIndex` (0.6.0) uses `std::sync::OnceLock` for its
/// `permutation_vanishing_polynomial_m` / `w` fields. Build a pre-filled `OnceLock`.
fn once_lock_with<T>(value: T) -> std::sync::OnceLock<T> {
    let cell = std::sync::OnceLock::new();
    let _ = cell.set(value);
    cell
}
use poly_commitment::ipa::SRS;
use poly_commitment::SRS as _;

/// Parse and finalize a blockchain verifier index from its JSON form (the
/// `*_blockchain_verifier_index.json` format with hex-string fields).
pub fn verifier_index_from_json(json: &str) -> Result<VerifierIndex<Fq>, serde_json::Error> {
    let index: VerifierIndex<Fq> = serde_json::from_str(json)?;
    Ok(finalize(index))
}

/// Detect which JSON encoding a blockchain verifier index is in and parse it.
///
/// - The OCaml `print_blockchain_snark_vk` tool emits `Pickles.Verification_key`
///   (`{ "commitments", "index", "data" }`) — used to bring in a hardfork VK (mesa)
///   that openmina can't generate.
/// - openmina/mina-tree emits the kimchi `VerifierIndex` directly.
///
/// `MINA_VK_JSON` / [`crate::Verifier::with_index_json`] accept either.
pub fn verifier_index_auto(json: &str) -> Result<VerifierIndex<Fq>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if v.get("commitments").is_some() && v.get("index").is_some() {
        pickles::verifier_index_from_pickles_json(json)
    } else {
        verifier_index_from_json(json).map_err(|e| e.to_string())
    }
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
        permutation_vanishing_polynomial_m: once_lock_with(permutation_vanishing_polynomial_m),
        w: once_lock_with(w),
        endo,
        linearization,
        powers_of_alpha,
        ..index
    }
}

/// Ingest the OCaml `print_blockchain_snark_vk` output (the
/// `Pickles.Verification_key.to_yojson` form) into a kimchi [`VerifierIndex`].
///
/// openmina cannot generate a hardfork's blockchain VK (it only loads precomputed
/// devnet/mainnet circuit blobs), so a hardfork (mesa) VK is produced by Mina's OCaml
/// tool and ingested here. The OCaml form encodes field elements as canonical
/// **big-endian** hex (vs mina-tree's little-endian) and curve points as `[x, y]`
/// affine pairs (vs compressed); we parse those into ark types, assemble the index,
/// and reuse [`finalize`] for the SRS/lagrange/linearization/permutation parts.
mod pickles {
    use ark_ec::AffineRepr;
    use ark_ff::PrimeField;
    use ark_poly::{EvaluationDomain, Radix2EvaluationDomain};
    use kimchi::circuits::wires::{COLUMNS, PERMUTS};
    use mina_curves::pasta::{Fp, Fq, Pallas};
    use mina_tree::proofs::VerifierIndex;
    use poly_commitment::commitment::PolyComm;
    use serde::Deserialize;

    type G = Pallas; // = <Fq as FieldWitness>::OtherCurve; commitment-point curve

    // The OCaml `{ commitments, index, data }` shape. We read the `commitments`
    // (PlonkVerificationKeyEvals — points as `[x,y]`) plus the scalar fields in
    // `index`; the blockchain circuit uses no optional gates, so those are absent.
    #[derive(Deserialize)]
    struct PicklesVk {
        commitments: Commitments,
        index: Index,
    }
    #[derive(Deserialize)]
    struct Commitments {
        sigma_comm: Vec<[String; 2]>,
        coefficients_comm: Vec<[String; 2]>,
        generic_comm: [String; 2],
        psm_comm: [String; 2],
        complete_add_comm: [String; 2],
        mul_comm: [String; 2],
        emul_comm: [String; 2],
        endomul_scalar_comm: [String; 2],
    }
    #[derive(Deserialize)]
    struct Index {
        domain: Domain,
        max_poly_size: usize,
        public: usize,
        prev_challenges: usize,
        zk_rows: u64,
        shifts: Vec<String>,
    }
    #[derive(Deserialize)]
    struct Domain {
        log_size_of_group: u32,
    }

    /// Parse a canonical big-endian `0x…` hex field element.
    fn field<F: PrimeField>(s: &str) -> Result<F, String> {
        let bytes = hex::decode(s.trim_start_matches("0x")).map_err(|e| e.to_string())?;
        Ok(F::from_be_bytes_mod_order(&bytes))
    }

    /// Parse an affine `[x, y]` (coords in `Fp`) into an on-curve commitment chunk.
    fn comm(xy: &[String; 2]) -> Result<PolyComm<G>, String> {
        let x: Fp = field(&xy[0])?;
        let y: Fp = field(&xy[1])?;
        let p = G::new_unchecked(x, y);
        if !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve() {
            return Err("commitment point not on curve".to_string());
        }
        Ok(PolyComm { chunks: vec![p] })
    }

    fn comms<const N: usize>(v: &[[String; 2]]) -> Result<[PolyComm<G>; N], String> {
        let parsed = v.iter().map(comm).collect::<Result<Vec<_>, _>>()?;
        parsed
            .try_into()
            .map_err(|_| format!("expected {N} commitments"))
    }

    pub fn verifier_index_from_pickles_json(json: &str) -> Result<VerifierIndex<Fq>, String> {
        let vk: PicklesVk = serde_json::from_str(json).map_err(|e| e.to_string())?;

        // A valid kimchi index to borrow the `#[serde(skip)]` fields from; `finalize`
        // recomputes all of them for the actual domain, so the values are placeholders.
        let template: VerifierIndex<Fq> =
            serde_json::from_str(crate::Verifier::embedded_index_json("devnet").unwrap())
                .map_err(|e| e.to_string())?;

        let domain = Radix2EvaluationDomain::<Fq>::new(1usize << vk.index.domain.log_size_of_group)
            .ok_or_else(|| "invalid domain size".to_string())?;

        let shift: [Fq; PERMUTS] = vk
            .index
            .shifts
            .iter()
            .map(|s| field::<Fq>(s))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| format!("expected {PERMUTS} shifts"))?;

        let index = VerifierIndex::<Fq> {
            domain,
            max_poly_size: vk.index.max_poly_size,
            zk_rows: vk.index.zk_rows,
            public: vk.index.public,
            prev_challenges: vk.index.prev_challenges,
            sigma_comm: comms::<PERMUTS>(&vk.commitments.sigma_comm)?,
            coefficients_comm: comms::<COLUMNS>(&vk.commitments.coefficients_comm)?,
            generic_comm: comm(&vk.commitments.generic_comm)?,
            psm_comm: comm(&vk.commitments.psm_comm)?,
            complete_add_comm: comm(&vk.commitments.complete_add_comm)?,
            mul_comm: comm(&vk.commitments.mul_comm)?,
            emul_comm: comm(&vk.commitments.emul_comm)?,
            endomul_scalar_comm: comm(&vk.commitments.endomul_scalar_comm)?,
            range_check0_comm: None,
            range_check1_comm: None,
            foreign_field_add_comm: None,
            foreign_field_mul_comm: None,
            xor_comm: None,
            rot_comm: None,
            shift,
            lookup_index: None,
            ..template
        };
        Ok(super::finalize(index))
    }
}
