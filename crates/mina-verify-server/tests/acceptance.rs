//! End-to-end acceptance tests: spawn the real `mina-verify-server` binary on an ephemeral
//! port and drive it over a socket. The dependency-free HTTP/1.1 client below keeps the
//! crate's dep set unchanged.
//!
//! The fast tests (health / bad-request / not-found) run on every `cargo test`. The two
//! real-proof tests are `#[ignore]`d — a SNARK verification is seconds in release and
//! minutes in debug — so run them with:
//!
//! ```text
//! cargo test -p mina-verify-server --release -- --ignored
//! ```

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// A spawned server bound to `addr`, killed on drop.
struct Server {
    child: Child,
    addr: String,
}

impl Server {
    fn start() -> Server {
        // Grab a free port by binding to :0, then hand it to the server. (Tiny race
        // between drop and the child's bind, acceptable for a test.)
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let addr = format!("127.0.0.1:{port}");

        let child = Command::new(env!("CARGO_BIN_EXE_mina-verify-server"))
            .env("BIND", &addr)
            .env("MINA_NETWORK", "devnet")
            .env("VERIFY_THREADS", "2")
            .env("RUST_LOG", "warn")
            .spawn()
            .expect("spawn mina-verify-server");

        let server = Server { child, addr };
        server.wait_ready();
        server
    }

    /// Poll until the server accepts connections (it parses the embedded VK at startup —
    /// fast in release, but tens of seconds in an unoptimized debug build).
    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(180);
        while Instant::now() < deadline {
            if TcpStream::connect(&self.addr).is_ok() {
                // Confirm it actually answers, not just that the port is open.
                if let Ok((200, _)) = self.get("/health") {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("server did not become ready within 180s");
    }

    fn get(&self, path: &str) -> std::io::Result<(u16, String)> {
        self.request("GET", path, None)
    }

    fn post(&self, path: &str, body: &str) -> std::io::Result<(u16, String)> {
        self.request("POST", path, Some(body))
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> std::io::Result<(u16, String)> {
        let mut stream = TcpStream::connect(&self.addr)?;
        let body = body.unwrap_or("");
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            self.addr,
            body.len()
        );
        stream.write_all(req.as_bytes())?;
        stream.flush()?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw)?;
        let text = String::from_utf8_lossy(&raw);
        let status: u16 = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .expect("status line");
        let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        Ok((status, body))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn fixture() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/devnet-528700.json"
    );
    let bytes = std::fs::read(path).expect("read fixture block");
    // Real precomputed blocks are not strictly UTF-8 — decode lossily, as the server does.
    String::from_utf8_lossy(&bytes).into_owned()
}

/// All the fast (no-proof) endpoint checks against a single live server. Combined into
/// one test so the ~tens-of-seconds debug-mode startup is paid once, not per assertion.
#[test]
fn fast_endpoints_behave() {
    let server = Server::start();

    // GET /health
    let (status, body) = server.get("/health").unwrap();
    assert_eq!(status, 200, "health: {body}");
    assert!(body.contains("\"status\":\"ok\""), "health body: {body}");
    assert!(
        body.contains("\"network\":\"devnet\""),
        "health body: {body}"
    );

    // POST /verify with non-JSON → 400 valid:false
    let (status, body) = server.post("/verify", "definitely not a block").unwrap();
    assert_eq!(status, 400, "malformed: {body}");
    assert!(body.contains("\"valid\":false"), "malformed body: {body}");

    // POST /verify with an empty object → 400 valid:false (missing protocol_state)
    let (status, body) = server.post("/verify", "{}").unwrap();
    assert_eq!(status, 400, "empty: {body}");
    assert!(body.contains("\"valid\":false"), "empty body: {body}");

    // Unknown route → 404
    let (status, _) = server.get("/does-not-exist").unwrap();
    assert_eq!(status, 404);
}

#[test]
#[ignore = "heavy: runs a real SNARK verification — run with `--release -- --ignored`"]
fn real_block_verifies_with_correct_facts() {
    let server = Server::start();
    let (status, body) = server.post("/verify", &fixture()).unwrap();
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"valid\":true"), "body: {body}");
    assert!(body.contains("\"height\":528700"), "body: {body}");
    assert!(
        body.contains("3NKAteSXBXDELWVeTy3xLRe1WEPNzopApQfm21FK5PquH3H4xDks"),
        "state_hash missing: {body}"
    );
}

#[test]
#[ignore = "heavy: runs a real SNARK verification — run with `--release -- --ignored`"]
fn tampered_block_is_rejected() {
    let server = Server::start();
    // Mutate the proven consensus state (block height): the block still decodes, but its
    // proof no longer matches the public input, so verification must return valid:false.
    let mut v: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    v["data"]["protocol_state"]["body"]["consensus_state"]["blockchain_length"] =
        serde_json::json!("999999");
    let (status, body) = server.post("/verify", &v.to_string()).unwrap();
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"valid\":false"), "body: {body}");
}
