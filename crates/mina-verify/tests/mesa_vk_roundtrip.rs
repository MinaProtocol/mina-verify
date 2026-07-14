//! The V2 control passes (Berkeley: 228,174 accounts, exact root) but Berkeley has *no*
//! zkApp accounts -- so the shared path is proven and mesa's 1,818 zkApps are the only
//! place left for the bug.
//!
//! Inside a zkApp, the verification key is the piece most likely to have changed shape in
//! tx-v3: it arrives as base64 binprot and we decode it with the *V2* wire type. If that
//! type is wrong for mesa, the decode can succeed and still yield the wrong key -- and the
//! wrong account hash. A faithful decode must round-trip back to the exact same base64.

use mina_p2p_messages::v2::MinaBaseVerificationKeyWireStableV1;
use mina_tree::VerificationKey;
use serde_json::Value;

#[test]
#[ignore = "needs MESA_GENESIS_LEDGER"]
fn mesa_verification_keys_round_trip() {
    let path = std::env::var("MESA_GENESIS_LEDGER").expect("set MESA_GENESIS_LEDGER");
    let file = std::fs::File::open(&path).expect("open");
    let genesis: Value = serde_json::from_reader(std::io::BufReader::new(file)).expect("parse");

    let accounts = genesis["ledger"]["accounts"].as_array().expect("accounts");

    let (mut checked, mut broken) = (0usize, 0usize);

    for account in accounts {
        let Some(vk_b64) = account["zkapp"]["verification_key"].as_str() else {
            continue;
        };

        let wire = match MinaBaseVerificationKeyWireStableV1::from_base64(vk_b64) {
            Ok(w) => w,
            Err(e) => {
                println!("DECODE FAILED: {e:?}");
                broken += 1;
                continue;
            }
        };

        let vk = VerificationKey::try_from(&wire).expect("wire -> VerificationKey");
        let wire_again = MinaBaseVerificationKeyWireStableV1::from(&vk);
        let b64_again = wire_again.to_base64().expect("re-encode");

        if b64_again != vk_b64 {
            if broken < 3 {
                println!("ROUND-TRIP MISMATCH");
                println!(
                    "  in  (len {}): {}",
                    vk_b64.len(),
                    &vk_b64[..80.min(vk_b64.len())]
                );
                println!(
                    "  out (len {}): {}",
                    b64_again.len(),
                    &b64_again[..80.min(b64_again.len())]
                );
            }
            broken += 1;
        }

        checked += 1;
    }

    println!("\nverification keys checked : {checked}");
    println!("round-trip mismatches     : {broken}");

    assert_eq!(
        broken, 0,
        "mesa verification keys do not survive a decode/encode round-trip -- the V2 wire \
         type does not describe them, so every zkApp account hashes wrong"
    );
}
