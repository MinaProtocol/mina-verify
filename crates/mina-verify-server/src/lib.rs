//! **Verify sidecar** — a long-lived HTTP service that answers one question:
//! "is this precomputed block's proof honest?"
//!
//! A trustless indexer (or any consumer) POSTs a precomputed-block JSON; the service
//! verifies its Pickles/kimchi SNARK proof and returns the proof-backed facts. A valid
//! proof attests the entire chain to genesis by recursion, so the caller can gate
//! ingestion on the answer without trusting whoever produced the block.
//!
//! Why a service and not a CLI: the expensive part is building the [`Verifier`] (loading
//! the verification key + standing up the proof system) — paid **once** at startup here,
//! so each request is just the proof check. The service is stateless and pure, so run as
//! many replicas as you like.
//!
//! ## Endpoints
//! - `GET  /health` → `{ "status": "ok", "network": "<name>" }`
//! - `POST /verify`  (body = precomputed-block JSON) →
//!   - proof valid:   `200 { "valid": true,  "height", "state_hash",
//!     "previous_state_hash", "staged_ledger_hash" }`
//!   - proof invalid: `200 { "valid": false, "error": "block proof did not verify" }`
//!   - undecodable:   `400 { "valid": false, "error": "<detail>" }`
//!
//! ## Config (env)
//! - `BIND` — listen address (default `0.0.0.0:8090`)
//! - `MINA_VK_JSON` — path to a blockchain verifier-index JSON (any network; required for
//!   mesa / mesa-mut, which have no embedded VK). Takes precedence.
//! - `MINA_NETWORK` — embedded-VK network when `MINA_VK_JSON` is unset (`devnet` /
//!   `mainnet`; default `devnet`).
//! - `VERIFY_THREADS` — worker threads (default: available parallelism). Verification is
//!   CPU-bound, so this caps concurrent in-flight verifies.

use std::io::Read;
use std::sync::Arc;

use mina_verify::{Verifier, VerifierError};
use tiny_http::{Header, Request, Response, Server};

// Re-exported so integration tests (and callers) can name the router's method type
// without depending on `tiny_http` directly.
pub use tiny_http::Method;

/// Max request body we will buffer. Real precomputed blocks are ~1 MB; this cap sits
/// well above that so a single oversized (or malicious) POST can't exhaust memory.
/// A body past the cap is truncated, which makes the block undecodable → a clean 400.
pub const MAX_BODY_BYTES: u64 = 32 * 1024 * 1024;

/// Default listen address when `BIND` is unset.
pub const DEFAULT_BIND: &str = "0.0.0.0:8090";

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Build the verifier once: an explicit VK JSON wins (covers mesa / regenerated
/// mainnet), otherwise the embedded VK for `MINA_NETWORK`.
pub fn build_verifier() -> Result<Verifier, String> {
    if let Ok(path) = std::env::var("MINA_VK_JSON") {
        let json = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read MINA_VK_JSON {path:?}: {e}"))?;
        Verifier::with_index_json(&json).map_err(|e| e.to_string())
    } else {
        let network = env_or("MINA_NETWORK", "devnet");
        Verifier::for_network_offline(&network).map_err(|e| e.to_string())
    }
}

fn json_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).expect("valid header")
}

fn respond(req: Request, status: u16, body: serde_json::Value) {
    let data = body.to_string();
    let response = Response::from_string(data)
        .with_status_code(status)
        .with_header(json_header());
    if let Err(e) = req.respond(response) {
        log::warn!("failed to send response: {e}");
    }
}

/// Verify a precomputed-block JSON body and turn the result into (status, json).
pub fn verify_body(verifier: &Verifier, body: &str) -> (u16, serde_json::Value) {
    match verifier.verify_precomputed_and_extract(body) {
        Ok(vb) => (
            200,
            serde_json::json!({
                "valid": true,
                "height": vb.height,
                "state_hash": vb.state_hash.to_string(),
                "previous_state_hash": vb.previous_state_hash.to_string(),
                "staged_ledger_hash": vb.staged_ledger_hash.to_string(),
            }),
        ),
        // Proof checked and was rejected — a well-formed "no". Caller must NOT ingest.
        Err(VerifierError::ProofInvalid) => (
            200,
            serde_json::json!({ "valid": false, "error": "block proof did not verify" }),
        ),
        // Anything else (couldn't decode the block / VK) is a bad request.
        Err(e) => (
            400,
            serde_json::json!({ "valid": false, "error": e.to_string() }),
        ),
    }
}

/// Pure request router: maps (method, path, body) → (status, json). Holds no I/O, so it
/// is unit-testable without a socket. The verify branch is wrapped in `catch_unwind`:
/// the body is *untrusted* input and a malformed-but-decodable block could trip an
/// assertion deep in proof verification — that must fail one request, not kill a worker.
pub fn dispatch(
    verifier: &Verifier,
    method: &Method,
    path: &str,
    body: &str,
) -> (u16, serde_json::Value) {
    match (method, path) {
        (Method::Get, "/health") => (
            200,
            serde_json::json!({ "status": "ok", "network": verifier.network() }),
        ),
        (Method::Post, "/verify") => {
            let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                verify_body(verifier, body)
            }));
            guarded.unwrap_or_else(|_| {
                log::error!("panic while verifying a request body; returning 500");
                (
                    500,
                    serde_json::json!({ "valid": false, "error": "internal verifier error" }),
                )
            })
        }
        _ => (
            404,
            serde_json::json!({ "error": "not found; use GET /health or POST /verify" }),
        ),
    }
}

fn handle(verifier: &Verifier, mut req: Request) {
    let method = req.method().clone();
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or(&url).to_string();

    // Read the (capped) body once, here, so `dispatch` stays pure. Real precomputed
    // blocks are NOT strictly UTF-8: the OCaml daemon emits some byte-string fields
    // (e.g. `sok_digest` inside `staged_ledger_diff`) as mixed raw/escaped bytes. Those
    // fields are ignored by verification (only `protocol_state` + the proof, both ASCII,
    // are read), so decode lossily — matching how JS clients read these blocks.
    let mut bytes = Vec::new();
    if let Err(e) = req.as_reader().take(MAX_BODY_BYTES).read_to_end(&mut bytes) {
        respond(
            req,
            400,
            serde_json::json!({ "valid": false, "error": format!("reading body: {e}") }),
        );
        return;
    }
    let body = String::from_utf8_lossy(&bytes);

    let (status, json) = dispatch(verifier, &method, &path, &body);
    respond(req, status, json);
}

/// Pick the worker-thread count from `VERIFY_THREADS`, else available parallelism.
pub fn worker_threads() -> usize {
    std::env::var("VERIFY_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        })
}

/// Run the server: bind, spawn `threads` workers each draining the shared listener, and
/// block until they exit. Verification is blocking CPU work, so a thread-per-worker pool
/// fits better than async.
pub fn serve(verifier: Arc<Verifier>, bind: &str, threads: usize) -> std::io::Result<()> {
    let server = Arc::new(
        Server::http(bind)
            .map_err(|e| std::io::Error::other(format!("cannot bind {bind}: {e}")))?,
    );

    log::info!(
        "mina-verify-server listening on {bind} (network={}, {threads} workers)",
        verifier.network()
    );

    let mut handles = Vec::with_capacity(threads);
    for _ in 0..threads {
        let server = Arc::clone(&server);
        let verifier = Arc::clone(&verifier);
        handles.push(std::thread::spawn(move || {
            for req in server.incoming_requests() {
                handle(&verifier, req);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

/// Process entry point: init logging, build the verifier, serve. Exits the process with
/// code 2 on a fatal startup error (bad VK, bind failure).
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let verifier = Arc::new(build_verifier().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(2);
    }));

    let bind = env_or("BIND", DEFAULT_BIND);
    let threads = worker_threads();

    if let Err(e) = serve(verifier, &bind, threads) {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}
