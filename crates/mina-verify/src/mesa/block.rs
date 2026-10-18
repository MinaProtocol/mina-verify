//! The staged ledger a block commits to, rebuilt from the block itself.
//!
//! A precomputed block carries `accounts_accessed`: for every account the block touched,
//! its **ledger index** and its **full post-state**. Apply those onto the parent ledger
//! and you have the ledger as of that block -- and its merkle root is the
//! `staged_ledger_hash` the block committed to, which the block's Pickles proof covers.
//! That is what lets an indexer's ledger be *verified* rather than trusted (see the module
//! docs in `ingest.rs`).
//!
//! The daemon writes accounts in `Account.to_yojson` shape, which differs from the genesis
//! state-dump shape that [`super::json::Account`] parses:
//!
//! | field | block (`to_yojson`) | genesis config |
//! |---|---|---|
//! | `balance`, timing amounts | nanomina integer string | MINA decimal string |
//! | permissions | `["Signature"]`, `[["Signature"], "3"]` | `"signature"`, `{auth, txn_version}` |
//! | `timing` | `["Untimed"]` / `["Timed", {..}]` | absent / object |
//! | zkApp `app_state`, `action_state` | `0x`-prefixed big-endian hex | decimal field element |
//! | zkApp `verification_key` | `{data: <base58check>, hash: <fp>}` | bare base64 binprot |
//! | `public_key`, `token_id` | those names | `pk`, `token` |
//!
//! Rather than write a second account parser -- a second chance to pack a field wrong --
//! this rewrites the block's JSON into the config shape and hands it to the deserializer
//! that already reproduces the mesa genesis root exactly.

use super::json::Account as JsonAccount;
use serde_json::{json, Map, Value};
use std::fmt::{self, Display};

/// A block's account JSON could not be rewritten into the genesis-config shape.
#[derive(Debug)]
pub enum BlockAccountError {
    MissingField(&'static str),
    BadShape { field: &'static str, value: String },
    MalformedKey(String),
    Json(String),
}

impl Display for BlockAccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => {
                write!(f, "account is missing the required field {field:?}")
            }
            Self::BadShape { field, value } => {
                write!(f, "field {field:?} has unexpected shape: {value}")
            }
            Self::MalformedKey(why) => {
                write!(f, "malformed base58check verification key: {why}")
            }
            Self::Json(why) => write!(f, "malformed account: {why}"),
        }
    }
}

impl std::error::Error for BlockAccountError {}

impl From<serde_json::Error> for BlockAccountError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e.to_string())
    }
}

type Result<T> = std::result::Result<T, BlockAccountError>;

const PERMISSION_KEYS: [&str; 12] = [
    "edit_state",
    "access",
    "send",
    "receive",
    "set_delegate",
    "set_permissions",
    "set_zkapp_uri",
    "edit_action_state",
    "set_token_symbol",
    "increment_nonce",
    "set_voting_for",
    "set_timing",
];

const B58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

fn field<'a>(account: &'a Value, name: &'static str) -> Result<&'a Value> {
    account
        .get(name)
        .ok_or(BlockAccountError::MissingField(name))
}

fn bad(field: &'static str, value: &Value) -> BlockAccountError {
    BlockAccountError::BadShape {
        field,
        value: value.to_string(),
    }
}

/// nanomina integer string -> MINA decimal string, which is what the config shape uses.
fn to_mina(value: &Value, name: &'static str) -> Result<String> {
    let nanomina: u64 = match value {
        Value::String(s) => s.parse().map_err(|_| bad(name, value))?,
        Value::Number(n) => n.as_u64().ok_or_else(|| bad(name, value))?,
        _ => return Err(bad(name, value)),
    };

    Ok(format!(
        "{}.{:09}",
        nanomina / 1_000_000_000,
        nanomina % 1_000_000_000
    ))
}

/// `["Signature"]` -> `"signature"` (a bare string is tolerated too).
fn to_auth(value: &Value, name: &'static str) -> Result<String> {
    let tag = match value {
        Value::Array(items) => items.first().ok_or_else(|| bad(name, value))?,
        other => other,
    };

    tag.as_str()
        .map(str::to_lowercase)
        .ok_or_else(|| bad(name, value))
}

/// `0x…` big-endian hex -> the decimal field element the config shape uses.
fn to_field_element(value: &Value, name: &'static str) -> Result<String> {
    let s = value.as_str().ok_or_else(|| bad(name, value))?;

    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        // the field is 255 bits, so it does not fit a u128 -- go through a big integer
        Some(hex) => {
            let mut digits: Vec<u8> = vec![0];

            for ch in hex.chars() {
                let nibble = ch.to_digit(16).ok_or_else(|| bad(name, value))? as u8;
                let mut carry = nibble;

                // digits hold one decimal digit each, little-endian: multiply by 16, add
                for digit in digits.iter_mut() {
                    let product = *digit * 16 + carry;
                    *digit = product % 10;
                    carry = product / 10;
                }
                while carry > 0 {
                    digits.push(carry % 10);
                    carry /= 10;
                }
            }

            Ok(digits
                .iter()
                .rev()
                .map(|d| char::from(b'0' + d))
                .collect::<String>())
        }
        None => Ok(s.to_owned()),
    }
}

/// Blocks base58check-encode the verification key; the config shape carries the raw
/// binprot in base64. Strip the version byte and the 4-byte checksum, re-encode.
fn vk_to_base64(b58: &str) -> Result<String> {
    let mut bytes: Vec<u8> = vec![0];

    for ch in b58.chars() {
        let value = B58
            .iter()
            .position(|&c| c == ch as u8)
            .ok_or_else(|| BlockAccountError::MalformedKey(format!("bad base58 char {ch:?}")))?;
        let mut carry = value;

        for byte in bytes.iter_mut() {
            let product = *byte as usize * 58 + carry;
            *byte = (product % 256) as u8;
            carry = product / 256;
        }
        while carry > 0 {
            bytes.push((carry % 256) as u8);
            carry /= 256;
        }
    }

    // leading '1's are leading zero bytes
    for _ in b58.chars().take_while(|&c| c == '1') {
        bytes.push(0);
    }
    bytes.reverse();

    // [version][payload][checksum: 4]
    if bytes.len() <= 5 {
        return Err(BlockAccountError::MalformedKey(
            "too short to carry a version byte and a checksum".into(),
        ));
    }

    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes[1..bytes.len() - 4]))
}

fn convert_permissions(permissions: &Value) -> Result<Value> {
    let mut out = Map::new();

    for key in PERMISSION_KEYS {
        let value = permissions
            .get(key)
            .ok_or(BlockAccountError::MissingField("permissions field"))?;
        out.insert(
            key.to_owned(),
            Value::String(to_auth(value, "permissions")?),
        );
    }

    // [["Signature"], "3"] -> {"auth": "signature", "txn_version": "3"}
    let svk = field(permissions, "set_verification_key")?;
    let pair = svk
        .as_array()
        .ok_or_else(|| bad("set_verification_key", svk))?;
    let auth = pair
        .first()
        .ok_or_else(|| bad("set_verification_key", svk))?;
    let txn_version = pair
        .get(1)
        .ok_or_else(|| bad("set_verification_key", svk))?;

    out.insert(
        "set_verification_key".to_owned(),
        json!({
            "auth": to_auth(auth, "set_verification_key")?,
            "txn_version": txn_version.as_str().map(str::to_owned)
                .unwrap_or_else(|| txn_version.to_string()),
        }),
    );

    Ok(Value::Object(out))
}

/// `["Untimed"]` -> `None`; `["Timed", {..}]` -> the config object.
fn convert_timing(timing: &Value) -> Result<Option<Value>> {
    let items = match timing {
        Value::Array(items) => items,
        Value::Null => return Ok(None),
        other => return Err(bad("timing", other)),
    };

    match items.first().and_then(Value::as_str) {
        Some("Untimed") | None => Ok(None),
        Some("Timed") => {
            let t = items.get(1).ok_or_else(|| bad("timing", timing))?;

            Ok(Some(json!({
                "initial_minimum_balance":
                    to_mina(field(t, "initial_minimum_balance")?, "initial_minimum_balance")?,
                "cliff_time": field(t, "cliff_time")?.to_string().trim_matches('"').to_owned(),
                "cliff_amount": to_mina(field(t, "cliff_amount")?, "cliff_amount")?,
                "vesting_period":
                    field(t, "vesting_period")?.to_string().trim_matches('"').to_owned(),
                "vesting_increment":
                    to_mina(field(t, "vesting_increment")?, "vesting_increment")?,
            })))
        }
        Some(_) => Err(bad("timing", timing)),
    }
}

fn convert_zkapp(zkapp: &Value) -> Result<Option<Value>> {
    if zkapp.is_null() {
        return Ok(None);
    }

    let app_state = field(zkapp, "app_state")?
        .as_array()
        .ok_or_else(|| bad("app_state", zkapp))?
        .iter()
        .map(|fp| to_field_element(fp, "app_state"))
        .collect::<Result<Vec<_>>>()?;

    let action_state = field(zkapp, "action_state")?
        .as_array()
        .ok_or_else(|| bad("action_state", zkapp))?
        .iter()
        .map(|fp| to_field_element(fp, "action_state"))
        .collect::<Result<Vec<_>>>()?;

    let mut out = json!({
        "app_state": app_state,
        "action_state": action_state,
        "zkapp_version": field(zkapp, "zkapp_version")?,
        "last_action_slot": field(zkapp, "last_action_slot")?,
        "proved_state": field(zkapp, "proved_state")?,
        "zkapp_uri": field(zkapp, "zkapp_uri")?,
    });

    // {"data": <base58check>, "hash": <fp>} -> the bare base64 binprot key. It is hashed
    // into the account, so dropping it would silently give every zkApp the dummy vk.
    if let Some(vk) = zkapp.get("verification_key") {
        if !vk.is_null() {
            let data = vk
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("verification_key", vk))?;

            out["verification_key"] = Value::String(vk_to_base64(data)?);
        }
    }

    Ok(Some(out))
}

/// Rewrite one `accounts_accessed` account into the genesis-config shape.
pub fn config_account_from_block(account: &Value) -> Result<Value> {
    let mut out = Map::new();

    out.insert("pk".to_owned(), field(account, "public_key")?.to_owned());
    out.insert("token".to_owned(), field(account, "token_id")?.to_owned());
    out.insert(
        "balance".to_owned(),
        Value::String(to_mina(field(account, "balance")?, "balance")?),
    );
    out.insert("nonce".to_owned(), field(account, "nonce")?.to_owned());
    out.insert(
        "receipt_chain_hash".to_owned(),
        field(account, "receipt_chain_hash")?.to_owned(),
    );
    out.insert(
        "voting_for".to_owned(),
        field(account, "voting_for")?.to_owned(),
    );
    out.insert(
        "permissions".to_owned(),
        convert_permissions(field(account, "permissions")?)?,
    );

    if let Some(symbol) = account.get("token_symbol") {
        out.insert("token_symbol".to_owned(), symbol.to_owned());
    }

    // an omitted delegate defaults to the account's own pk, which is NOT the same as the
    // block saying null -- carry it across only when the block does
    if let Some(delegate) = account.get("delegate") {
        if !delegate.is_null() {
            out.insert("delegate".to_owned(), delegate.to_owned());
        }
    }

    if let Some(timing) = account.get("timing") {
        if let Some(timing) = convert_timing(timing)? {
            out.insert("timing".to_owned(), timing);
        }
    }

    if let Some(zkapp) = account.get("zkapp") {
        if let Some(zkapp) = convert_zkapp(zkapp)? {
            out.insert("zkapp".to_owned(), zkapp);
        }
    }

    Ok(Value::Object(out))
}

/// Every account a block touched, as `(ledger index, account)` in the genesis-config shape.
///
/// `block` is the precomputed block JSON (the GCS object, `{"data": {..}, "version": _}`).
/// Apply these onto the parent ledger, in block order, and the ledger's merkle root is the
/// block's `staged_ledger_hash`.
pub fn accounts_accessed(block: &Value) -> Result<Vec<(usize, JsonAccount)>> {
    let data = block.get("data").unwrap_or(block);
    let accessed = data
        .get("accounts_accessed")
        .and_then(Value::as_array)
        .ok_or(BlockAccountError::MissingField("accounts_accessed"))?;

    accessed
        .iter()
        .map(|entry| {
            let pair = entry
                .as_array()
                .ok_or_else(|| bad("accounts_accessed entry", entry))?;
            let index =
                pair.first()
                    .and_then(Value::as_u64)
                    .ok_or_else(|| bad("accounts_accessed index", entry))? as usize;
            let account = pair
                .get(1)
                .ok_or_else(|| bad("accounts_accessed account", entry))?;

            let config = config_account_from_block(account)?;
            let account: JsonAccount = serde_json::from_value(config)?;

            Ok((index, account))
        })
        .collect()
}

/// The `staged_ledger_hash` the block commits to (base58, `jw…`/`jx…`).
pub fn staged_ledger_hash(block: &Value) -> Option<&str> {
    block
        .get("data")
        .unwrap_or(block)
        .get("protocol_state")?
        .get("body")?
        .get("blockchain_state")?
        .get("staged_ledger_hash")?
        .get("non_snark")?
        .get("ledger_hash")?
        .as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nanomina_becomes_a_mina_decimal_string() {
        assert_eq!(
            to_mina(&json!("99011147762714000"), "balance").unwrap(),
            "99011147.762714000"
        );
        assert_eq!(to_mina(&json!("0"), "balance").unwrap(), "0.000000000");
        assert_eq!(to_mina(&json!("1"), "balance").unwrap(), "0.000000001");
    }

    #[test]
    fn permission_variants_become_config_strings() {
        assert_eq!(to_auth(&json!(["Signature"]), "p").unwrap(), "signature");
        assert_eq!(to_auth(&json!(["None"]), "p").unwrap(), "none");
        assert_eq!(to_auth(&json!("Proof"), "p").unwrap(), "proof");
    }

    #[test]
    fn hex_field_elements_become_decimal() {
        assert_eq!(
            to_field_element(
                &json!("0x0000000000000000000000000000000000000000000000000000000000000064"),
                "f"
            )
            .unwrap(),
            "100"
        );
        // already decimal (the genesis-config shape) passes through untouched
        assert_eq!(to_field_element(&json!("100"), "f").unwrap(), "100");

        // a 255-bit element -- the whole reason this cannot go through a u128
        assert_eq!(
            to_field_element(
                &json!("0x3772BC5435B957F81F86F752E93F2E29E886AC24580B3D1EC879C1DAD26965F9"),
                "f"
            )
            .unwrap(),
            "25079927036070901246064867767436987657692091363973573142121686150614948079097"
        );
    }

    #[test]
    fn untimed_accounts_carry_no_timing() {
        assert!(convert_timing(&json!(["Untimed"])).unwrap().is_none());
    }

    #[test]
    fn timed_accounts_carry_mina_denominated_amounts() {
        let timing = convert_timing(&json!([
            "Timed",
            {
                "initial_minimum_balance": "23063715308540",
                "cliff_time": "449662",
                "cliff_amount": "95486111",
                "vesting_period": "2",
                "vesting_increment": "95486111",
            }
        ]))
        .unwrap()
        .expect("timed");

        assert_eq!(timing["initial_minimum_balance"], "23063.715308540");
        assert_eq!(timing["cliff_amount"], "0.095486111");
        assert_eq!(timing["cliff_time"], "449662");
    }
}
