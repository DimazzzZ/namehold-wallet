//! The listing file is Shakedex's v2 auction JSON — what LearnHNS Market
//! serves at `/listing/<name>/proof.json`, Bob imports and the CLI writes.
//! Read strictly where it protects a buyer (version 2 only, sighash byte
//! exactly 0x84, this network's payment address) and leniently where wallets
//! in the wild differ (feeAddr with a zero fee or unusable here, unknown
//! fields kept).

use crate::noncustodial::shakedex::purchase::MAX_MONEY;
use crate::noncustodial::shakedex::template::STEP_SIGHASH;
use serde::Deserialize;
use serde_json::Value;

use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::network::Network;
use crate::noncustodial::tx::{output_address_from_string, OutputAddress};

pub const MAX_LISTING_FILE_BYTES: usize = 256 * 1024;
const VERSION: u64 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct PriceStep {
    pub price: u64,
    pub lock_time: u64,
    pub signature: [u8; 65],
    pub fee: u64,
}

#[derive(Clone, Debug)]
pub struct ListingFile {
    pub name: String,
    pub lock_txid: [u8; 32],
    pub lock_vout: u32,
    pub public_key: [u8; 33],
    pub payment_addr: String,
    /// The fee address in effect: `None` when no step carries a fee.
    pub fee_addr: Option<String>,
    pub steps: Vec<PriceStep>,
    pub expires_at: Option<u64>,
    /// The file as written, for the round-trip (R2): what the fields above
    /// interpret (a `feeAddr` beside zero fees, an absent step `fee`, hex in
    /// either case, a null or ISO-8601 `expiresAt`) and every unknown field, at every
    /// level, come back unchanged.
    as_written: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    name: String,
    locking_tx_hash: String,
    locking_output_idx: u64,
    public_key: String,
    payment_addr: String,
    #[serde(default)]
    fee_addr: Option<String>,
    data: Vec<RawStep>,
    version: u64,
    #[serde(default)]
    expires_at: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawStep {
    price: u64,
    lock_time: u64,
    signature: String,
    #[serde(default)]
    fee: u64,
}

fn bad(msg: impl Into<String>) -> AppError {
    AppError::InvalidInput(format!("listing file: {}", msg.into()))
}

fn hex_array<const N: usize>(s: &str, field: &str) -> Result<[u8; N], AppError> {
    hex::decode(s)
        .map_err(|_| bad(format!("{field} is not hex")))?
        .try_into()
        .map_err(|_| bad(format!("{field} has the wrong length")))
}

/// `expiresAt` in seconds. A listing file (proof.json, the CLI) writes it as
/// an integer; the LearnHNS Market feed (`/api/v2/auctions`) writes the same
/// instant as a naive UTC ISO-8601 string, fractional seconds dropped.
fn expiry_seconds(v: &Value) -> Result<Option<u64>, AppError> {
    let refused = || bad("expiresAt is neither seconds nor an ISO-8601 UTC time");
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => n.as_u64().map(Some).ok_or_else(refused),
        Value::String(s) => {
            let t = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
                .map_err(|_| refused())?;
            u64::try_from(t.and_utc().timestamp())
                .map(Some)
                .map_err(|_| refused())
        }
        _ => Err(refused()),
    }
}

/// How a network is named to the user: "mainnet", not hsd's "main".
fn network_name(network: Network) -> &'static str {
    match network {
        Network::Main => "mainnet",
        other => other.as_str(),
    }
}

fn check_address(network: Network, addr: &str, field: &str) -> Result<(), AppError> {
    let (version, program) = address::decode(network, addr).map_err(|_| {
        // An address of another network means the whole listing is for that
        // network: say so, rather than that one field is malformed.
        let other = [
            Network::Main,
            Network::Testnet,
            Network::Regtest,
            Network::Simnet,
        ]
        .into_iter()
        .find(|n| *n != network && address::decode(*n, addr).is_ok());
        match other {
            Some(other) => AppError::InvalidInput(format!(
                "this listing is for {}, but this wallet is on {}: open it with a {} wallet",
                network_name(other),
                network_name(network),
                network_name(other)
            )),
            None => bad(format!(
                "{field} is not a {} address",
                network_name(network)
            )),
        }
    })?;
    if version != 0 || program.len() != 20 {
        return Err(bad(format!("{field} must be a version-0, 20-byte address")));
    }
    Ok(())
}

impl ListingFile {
    /// Each price step as `(price, encoded lock time)`, the form
    /// `template::current_step_index` reads.
    pub fn encoded_steps(&self) -> Result<Vec<(u64, u32)>, AppError> {
        self.steps
            .iter()
            .map(|s| {
                crate::noncustodial::shakedex::template::encode_lock_time(s.lock_time)
                    .map(|e| (s.price, e))
            })
            .collect()
    }

    pub fn parse(json: &str, network: Network) -> Result<Self, AppError> {
        if json.len() > MAX_LISTING_FILE_BYTES {
            return Err(bad(format!("larger than {MAX_LISTING_FILE_BYTES} bytes")));
        }
        let as_written: Value = serde_json::from_str(json).map_err(|e| bad(e.to_string()))?;
        let raw = Raw::deserialize(&as_written).map_err(|e| bad(e.to_string()))?;
        if raw.version != VERSION {
            return Err(bad(format!(
                "version {} is not supported (only 2)",
                raw.version
            )));
        }
        if !crate::noncustodial::names::verify_name(&raw.name) {
            return Err(bad(format!(
                "name {:?} is not a valid Handshake name",
                raw.name
            )));
        }
        if raw.data.is_empty() {
            return Err(bad("no price steps"));
        }
        let expires_at = raw
            .expires_at
            .as_ref()
            .map(expiry_seconds)
            .transpose()?
            .flatten();
        let lock_vout = u32::try_from(raw.locking_output_idx)
            .map_err(|_| bad("lockingOutputIdx out of range"))?;
        check_address(network, &raw.payment_addr, "paymentAddr")?;
        let any_fee = raw.data.iter().any(|s| s.fee > 0);
        // A zero fee adds no output; the presign is identical. A feeAddr that
        // is unusable here (another network, not 20-byte version 0, garbage)
        // is kept as written but never paid: see `fee_output_address`.
        let fee_addr = raw.fee_addr.clone().filter(|_| any_fee);
        let steps = raw
            .data
            .iter()
            .map(|s| {
                let signature: [u8; 65] = hex_array(&s.signature, "signature")?;
                if u32::from(signature[64]) != STEP_SIGHASH {
                    return Err(bad(format!(
                        "price step sighash type {:#04x}, expected 0x84",
                        signature[64]
                    )));
                }
                crate::noncustodial::shakedex::template::low_s_signature(&signature).map_err(
                    |e| match e {
                        AppError::InvalidInput(m) => bad(m),
                        other => other,
                    },
                )?;
                if s.price == 0 {
                    return Err(bad("price step has a zero price"));
                }
                // R3: a lock time past 40 bits of seconds has no encoding, so
                // the step could never become valid.
                crate::noncustodial::shakedex::template::encode_lock_time(s.lock_time)
                    .map_err(|_| bad("price step lock time is out of range"))?;
                match s.price.checked_add(s.fee) {
                    Some(total) if total <= MAX_MONEY => {}
                    _ => return Err(bad("price step pays more than the money supply")),
                }
                Ok(PriceStep {
                    price: s.price,
                    lock_time: s.lock_time,
                    signature,
                    fee: s.fee,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListingFile {
            name: raw.name,
            lock_txid: hex_array(&raw.locking_tx_hash, "lockingTxHash")?,
            lock_vout,
            public_key: hex_array(&raw.public_key, "publicKey")?,
            payment_addr: raw.payment_addr,
            fee_addr,
            steps,
            expires_at,
            as_written,
        })
    }

    /// Whether the file as written names a fee address (`feeAddr` present
    /// and not null), whatever its fees: [`Self::fee_addr`] is `None` beside
    /// zero fees, but the file still says it.
    pub fn names_a_fee_address(&self) -> bool {
        self.as_written.get("feeAddr").is_some_and(|v| !v.is_null())
    }

    pub fn to_json(&self) -> Result<String, AppError> {
        serde_json::to_string(&self.as_written).map_err(AppError::from)
    }

    pub fn payment_output_address(&self, network: Network) -> Result<OutputAddress, AppError> {
        output_address_from_string(network, &self.payment_addr)
    }

    /// Where the market fee is paid, or `None` when the listing names no
    /// usable fee address (missing, another network, not 20-byte version 0):
    /// the seller's signature does not cover it, so such a listing is still
    /// buyable, just without a fee output.
    pub fn fee_output_address(&self, network: Network) -> Option<OutputAddress> {
        let a = self.fee_addr.as_deref()?;
        check_address(network, a, "feeAddr").ok()?;
        output_address_from_string(network, a).ok()
    }

    /// Test-only constructor: no fee address, no expiry, no unknown fields.
    #[cfg(test)]
    pub fn for_tests(
        name: &str,
        lock_txid: [u8; 32],
        lock_vout: u32,
        public_key: [u8; 33],
        payment_addr: &str,
        steps: Vec<PriceStep>,
    ) -> Self {
        ListingFile {
            name: name.to_owned(),
            lock_txid,
            lock_vout,
            public_key,
            payment_addr: payment_addr.to_owned(),
            fee_addr: None,
            expires_at: None,
            as_written: serde_json::json!({
                "name": name,
                "lockingTxHash": hex::encode(lock_txid),
                "lockingOutputIdx": lock_vout,
                "publicKey": hex::encode(public_key),
                "paymentAddr": payment_addr,
                "data": steps.iter().map(|s| serde_json::json!({
                    "price": s.price,
                    "lockTime": s.lock_time,
                    "signature": hex::encode(s.signature),
                    "fee": s.fee,
                })).collect::<Vec<_>>(),
                "version": VERSION,
            }),
            steps,
        }
    }
}

/// What [`write_listing_file`] writes: one of our own listings.
pub struct NewListingFile<'a> {
    pub name: &'a str,
    pub lock_txid: [u8; 32],
    pub lock_vout: u32,
    pub public_key: [u8; 33],
    pub payment_addr: &'a str,
    pub steps: &'a [PriceStep],
    /// Unix seconds (R23: the MTP at signing plus 365 days).
    pub expires_at: u64,
}

/// R23: our listing as Shakedex v2 JSON, with the keys the CLI writes and
/// LearnHNS serves, no market fee (`feeAddr: null`, every `fee` 0; a step
/// carrying one is refused) and `expiresAt` in seconds. The file is read back by
/// [`ListingFile::parse`] before it is returned: a file our own strict
/// reader refuses is never handed out.
pub fn write_listing_file(l: &NewListingFile, network: Network) -> Result<String, AppError> {
    if l.steps.iter().any(|s| s.fee != 0) {
        return Err(bad(
            "we never write a market fee: every step's fee must be 0",
        ));
    }
    let file = serde_json::json!({
        "data": l.steps.iter().map(|s| serde_json::json!({
            "fee": s.fee,
            "lockTime": s.lock_time,
            "price": s.price,
            "signature": hex::encode(s.signature),
        })).collect::<Vec<_>>(),
        "expiresAt": l.expires_at,
        "feeAddr": Value::Null,
        "lockingOutputIdx": l.lock_vout,
        "lockingTxHash": hex::encode(l.lock_txid),
        "name": l.name,
        "paymentAddr": l.payment_addr,
        "publicKey": hex::encode(l.public_key),
        "version": VERSION,
    });
    let json = serde_json::to_string(&file)?;
    // What is guarded is that the strict reader takes it; `to_json` hands
    // back the value parsed, so comparing it with `file` would say nothing.
    ListingFile::parse(&json, network)?;
    Ok(json)
}

/// R26: `stored`, one of our own listing files as stored, with `step`
/// appended to its price steps (the keys [`write_listing_file`] writes, fee
/// 0) and, when the file names no expiry (imported without one, R32),
/// `expires_at` (R23, Unix seconds). Every other field, known or not, at
/// every level, stays as written. The result is read back by
/// [`ListingFile::parse`]: its steps must be the stored ones and `step`.
/// Returns the file and the expiry it names.
pub fn add_step_to_listing_file(
    stored: &str,
    step: &PriceStep,
    expires_at: u64,
    network: Network,
) -> Result<(String, u64), AppError> {
    if step.fee != 0 {
        return Err(bad(
            "we never write a market fee: every step's fee must be 0",
        ));
    }
    let file = ListingFile::parse(stored, network)?;
    let mut v = file.as_written.clone();
    v.get_mut("data")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| bad("no price steps"))?
        .push(serde_json::json!({
            "fee": step.fee,
            "lockTime": step.lock_time,
            "price": step.price,
            "signature": hex::encode(step.signature),
        }));
    if file.expires_at.is_none() {
        v["expiresAt"] = expires_at.into();
    }
    let json = serde_json::to_string(&v)?;
    let back = ListingFile::parse(&json, network)?;
    let mut want = file.steps.clone();
    want.push(step.clone());
    // The strict reader takes it, and its steps are the stored ones and
    // `step`; the other fields are `v`'s as written (`to_json` hands back
    // the value parsed, so it is not compared).
    if back.steps != want {
        return Err(AppError::Other(
            "the listing file's steps did not read back as the stored ones and the new one".into(),
        ));
    }
    let expiry = back
        .expires_at
        .ok_or_else(|| AppError::Other("the listing file lost its expiry".into()))?;
    Ok((json, expiry))
}
