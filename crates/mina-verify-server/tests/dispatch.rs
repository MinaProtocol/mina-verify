//! Fast unit tests for the pure request router. They build a real devnet verifier — which
//! only parses the embedded VK, no SNARK proof runs — so they're fast in release; in debug
//! the VK parse is the one slow step, so it's built **once** and shared. The expensive
//! valid/invalid-proof paths live in `acceptance.rs` behind `#[ignore]`.

use std::sync::OnceLock;

use mina_verify::Verifier;
use mina_verify_server::{dispatch, Method};

fn verifier() -> &'static Verifier {
    static V: OnceLock<Verifier> = OnceLock::new();
    V.get_or_init(|| Verifier::for_network_offline("devnet").expect("embedded devnet VK"))
}

#[test]
fn health_reports_ok_and_network() {
    let (status, body) = dispatch(verifier(), &Method::Get, "/health", "");
    assert_eq!(status, 200);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["network"], "devnet");
}

#[test]
fn unknown_route_is_404() {
    let (status, body) = dispatch(verifier(), &Method::Get, "/nope", "");
    assert_eq!(status, 404);
    assert!(body.get("error").is_some());
}

#[test]
fn verify_rejects_non_json_body_as_400() {
    let (status, body) = dispatch(verifier(), &Method::Post, "/verify", "this is not json");
    assert_eq!(status, 400);
    assert_eq!(body["valid"], false);
    assert!(body.get("error").is_some());
}

#[test]
fn verify_rejects_empty_object_as_400() {
    let (status, body) = dispatch(verifier(), &Method::Post, "/verify", "{}");
    assert_eq!(status, 400);
    assert_eq!(body["valid"], false);
}

#[test]
fn verify_on_get_is_404_not_run() {
    // Only POST /verify verifies; GET /verify must not be treated as a verify request.
    let (status, _) = dispatch(verifier(), &Method::Get, "/verify", "{}");
    assert_eq!(status, 404);
}
