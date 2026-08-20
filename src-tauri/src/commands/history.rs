//! Full wallet action history from the local hsd node.
//!
//! Uses the node REST route `GET /tx/address/:addr` (requires `--index-tx`
//! and `--index-address`, both enabled for app-managed nodes at `node.rs:394`
//! -395) to fetch every transaction touching each of the wallet's derived
//! addresses, dedupes by txid, and classifies each transaction into an
//! `ActionRow` — plain Send / Receive plus every name-covenant action
//! (OPEN / BID / REVEAL / REDEEM / REGISTER / UPDATE / RENEW / TRANSFER /
//! FINALIZE / REVOKE).
//!
//! Attribution math (which outputs / inputs belong to the wallet) mirrors the
//! existing send-vs-receive logic in `read_cached_transactions`
//! (`db/queries.rs:1820-1875`). The critical simplification vs. block scanning
//! is that `/tx/address` returns fully-decoded inputs with a resolved
//! `coin { value, address, covenant }` (see hsd api-docs), so spend attribution
//! needs no extra `getrawtransaction` roundtrips.
//!
//! Covenant constants come from `noncustodial::sync` (verified against hsd
//! `lib/covenants/rules.js`). We rely on the numeric `covenant.type`, NOT the
//! symbolic `action` string, because the `POST /tx/address` bulk route omits
//! the string (and we may add bulk later); the numeric type is always present.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;
use tauri::State;

use crate::commands::read::resolve_profile;
use crate::db::queries;
use crate::error::AppError;
use crate::noncustodial::rpc::NodeRpcClient;
use crate::noncustodial::sync::{
    COV_BID, COV_CLAIM, COV_FINALIZE, COV_NONE, COV_OPEN, COV_REDEEM, COV_REGISTER, COV_RENEW,
    COV_REVEAL, COV_REVOKE, COV_TRANSFER, COV_UPDATE,
};
use crate::AppState;

/// A classified row in the wallet's action history.
///
/// One tx -> one row. Fields are `camelCase` on the wire to match the frontend
/// convention used by `RenewalRow` et al. in `commands/read.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRow {
    /// Transaction hash (hex).
    pub txid: String,
    /// One of: `send`, `receive`, `open`, `bid`, `reveal`, `redeem`, `register`,
    /// `update`, `renew`, `transfer`, `finalize`, `revoke`, `claim`, `other`.
    pub action: String,
    /// Human-readable name (BID exposes raw name in `items[2]`; other covenants
    /// leave this `None` and the frontend can resolve via `getnamebyhash` if
    /// needed).
    pub name: Option<String>,
    /// Covenant `items[0]` — the SHA3 nameHash (hex), when any name-covenant
    /// output is present in this tx.
    pub name_hash: Option<String>,
    /// Signed net value in dollarydoos that ACTUALLY LEFT (or ENTERED) the
    /// wallet, EXCLUDING the fee — the same "net external flow" rule as
    /// `netSpendDoos` in `src/lib/utils.ts:20-24`. Receive rows are
    /// `+received_by_us`; wallet-initiated rows are `-sent_to_external`
    /// (i.e. 0 for a name-covenant action that self-homes the locked value
    /// back onto our own new coin — a DNS UPDATE is NOT a spend of the name's
    /// locked value, and treating it as one contradicts the drafts card).
    /// Only genuine outward flows (plain send, TRANSFER to a buyer,
    /// FINALIZE to the transfer target) produce a non-zero magnitude.
    pub value_doos: i64,
    /// One of: `receive`, `send`, `internal` (name actions carry `internal`
    /// when only the wallet's own coins move).
    pub direction: String,
    /// Confirming block height, or `None` when unconfirmed (hsd returns -1).
    pub height: Option<i64>,
    /// Unix time (seconds) of the confirming block; `None` when unconfirmed.
    pub time: Option<i64>,
    /// `true` when `height` is a real block; `false` when the tx is still in
    /// the mempool (`height == -1`).
    pub confirmed: bool,
    /// For destination-facing rows (Send / TRANSFER), the counterparty address
    /// when we can pick a canonical one. Empty otherwise.
    pub counterparty: Option<String>,
}

/// Classify one decoded hsd tx (as returned by `/tx/address`) against the
/// wallet's own-address set. Pure function — the whole point of splitting it
/// out is testability without a running node.
pub fn classify_tx(tx: &serde_json::Value, our_addrs: &HashSet<String>) -> Option<ActionRow> {
    let txid = tx.get("hash").and_then(|v| v.as_str())?.to_string();

    // Height: hsd emits -1 for mempool; treat that as unconfirmed. `block` is
    // null in that case, too.
    let raw_height = tx.get("height").and_then(|v| v.as_i64()).unwrap_or(-1);
    let (height, confirmed) = if raw_height >= 0 {
        (Some(raw_height), true)
    } else {
        (None, false)
    };
    let time = tx.get("time").and_then(|v| v.as_i64()).filter(|&t| t > 0);

    let empty = Vec::new();
    let outputs = tx
        .get("outputs")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    let inputs = tx
        .get("inputs")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);

    // --- Attribution (mirrors queries.rs:1820-1875 for send/receive) --------
    let mut received_by_us: i64 = 0;
    let mut sent_to_external: i64 = 0;
    let mut first_external_addr: Option<String> = None;

    // Track any name-covenant output on this tx (there's at most one per tx in
    // Handshake auction/name flows). If several appear we pick the first
    // non-NONE — the covenant type wins the classification.
    let mut name_cov_type: u8 = COV_NONE;
    let mut name_hash_hex: Option<String> = None;
    let mut name_display: Option<String> = None;
    let mut name_cov_addr_is_ours = false;
    let mut name_cov_addr: Option<String> = None;

    for o in outputs {
        let value = o.get("value").and_then(|v| v.as_i64()).unwrap_or(0);
        let addr = o.get("address").and_then(|v| v.as_str()).unwrap_or("");
        let is_ours = !addr.is_empty() && our_addrs.contains(addr);
        if is_ours {
            received_by_us += value;
        } else if !addr.is_empty() {
            sent_to_external += value;
            if first_external_addr.is_none() {
                first_external_addr = Some(addr.to_string());
            }
        }

        if name_cov_type == COV_NONE {
            if let Some(cov) = o.get("covenant") {
                let kind = cov.get("type").and_then(|t| t.as_u64()).unwrap_or(0) as u8;
                if kind != COV_NONE {
                    name_cov_type = kind;
                    if let Some(items) = cov.get("items").and_then(|i| i.as_array()) {
                        if let Some(h) = items.first().and_then(|v| v.as_str()) {
                            if !h.is_empty() {
                                name_hash_hex = Some(h.to_ascii_lowercase());
                            }
                        }
                        // BID covenant carries the raw name at items[2] (see
                        // chain_scan.rs:162-167). Decode hex -> utf8.
                        if kind == COV_BID {
                            if let Some(raw_hex) = items.get(2).and_then(|v| v.as_str()) {
                                if let Ok(bytes) = hex::decode(raw_hex) {
                                    if let Ok(s) = String::from_utf8(bytes) {
                                        if !s.is_empty() {
                                            name_display = Some(s);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    name_cov_addr_is_ours = is_ours;
                    name_cov_addr = if addr.is_empty() {
                        None
                    } else {
                        Some(addr.to_string())
                    };
                }
            }
        }
    }

    let mut spends_ours = false;
    for i in inputs {
        // Non-coinbase inputs on `/tx/address` include a resolved `coin`.
        let coin_addr = i
            .get("coin")
            .and_then(|c| c.get("address"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !coin_addr.is_empty() && our_addrs.contains(coin_addr) {
            spends_ours = true;
        }
    }

    // Drop txs that don't actually touch this wallet. hsd returns everything
    // linked to the address — that's already filtered by the caller — but this
    // guards against stale addresses being reused.
    if !spends_ours && received_by_us == 0 && !name_cov_addr_is_ours {
        return None;
    }

    // --- Classification -----------------------------------------------------
    //
    // Value math is UNIFIED across every action, matching `netSpendDoos` in
    // src/lib/utils.ts: whatever actually left the wallet (excluding fee), or
    // whatever entered it. A name-covenant tx that re-homes the name's locked
    // value back onto our own new coin is net 0 — the locked HNS is still ours.
    let value_doos = if spends_ours {
        -sent_to_external
    } else {
        received_by_us
    };

    let (action, direction, counterparty) = if name_cov_type != COV_NONE {
        // A name-covenant output determines the ACTION LABEL; the amount
        // above already captured the true net flow (0 for self-homed).
        let label = match name_cov_type {
            COV_OPEN => "open",
            COV_BID => "bid",
            COV_REVEAL => "reveal",
            COV_REDEEM => "redeem",
            COV_REGISTER => "register",
            COV_UPDATE => "update",
            COV_RENEW => "renew",
            COV_TRANSFER => "transfer",
            COV_FINALIZE => "finalize",
            COV_REVOKE => "revoke",
            COV_CLAIM => "claim",
            _ => "other",
        };
        let dir = if spends_ours {
            "send"
        } else if name_cov_addr_is_ours {
            "receive"
        } else {
            "internal"
        };
        // TRANSFER's destination is the new owner (external address on the
        // covenant output). Note: the covenant output itself carries no HNS
        // out of the wallet from the *value* perspective (it holds the name's
        // own locked amount), so `value_doos` above is already 0 for a
        // wallet-initiated TRANSFER — the row still surfaces the recipient
        // via `counterparty` so the UI can display it.
        let cp = if name_cov_type == COV_TRANSFER && !name_cov_addr_is_ours {
            name_cov_addr
        } else {
            None
        };
        (label.to_string(), dir.to_string(), cp)
    } else if spends_ours {
        // Plain HNS spend (no name covenant on any output).
        let dir = if sent_to_external > 0 {
            "send"
        } else {
            "internal"
        };
        ("send".to_string(), dir.to_string(), first_external_addr)
    } else {
        ("receive".to_string(), "receive".to_string(), None)
    };

    Some(ActionRow {
        txid,
        action,
        name: name_display,
        name_hash: name_hash_hex,
        value_doos,
        direction,
        height,
        time,
        confirmed,
        counterparty,
    })
}

/// Load a wallet profile's derived addresses.
fn load_wallet_addresses(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT address FROM derived_addresses WHERE wallet_profile_id = ?1 ORDER BY address",
    )?;
    let rows = stmt.query_map(rusqlite::params![profile_id], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Full wallet action history from the local hsd node.
///
/// - Requires `--index-tx` + `--index-address` on the node (surfaces a specific
///   error otherwise; the UI can gate on this to show the "enable address
///   index" banner).
/// - Iterates each derived address, calls `/tx/address` per address, dedupes
///   by txid.
/// - Classifies each tx via [`classify_tx`] using the wallet's own-address set.
/// - Returns rows sorted newest-first (unconfirmed first, then by height desc).
#[tauri::command]
pub async fn read_action_history(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<Vec<ActionRow>, AppError> {
    let profile_id = match resolve_profile(&state, wallet_profile_id)? {
        Some(id) => id,
        None => return Ok(Vec::new()),
    };

    // Snapshot addresses + settings under a short DB lock; drop before .await.
    let (addresses, settings) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let addrs = load_wallet_addresses(&conn, &profile_id)?;
        let settings = queries::get_settings(&conn)?;
        (addrs, settings)
    };
    if addresses.is_empty() {
        return Ok(Vec::new());
    }

    let our: HashSet<String> = addresses.iter().cloned().collect();
    let node = NodeRpcClient::from_settings(&settings);

    // Dedupe by txid. Preserve one representative decoded tx per txid.
    let mut by_txid: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for addr in &addresses {
        // Gracefully degrade when the node lacks `--index-tx` / `--index-address`
        // or is on a different network: swallow per-address errors and return
        // whatever we could fetch. If EVERY address fails, the result is simply
        // empty — the UI shows "No activity yet" instead of an error toast.
        match node.get_txs_by_address(addr).await {
            Ok(txs) => {
                for tx in txs {
                    if let Some(hash) = tx.get("hash").and_then(|h| h.as_str()) {
                        by_txid.entry(hash.to_string()).or_insert(tx);
                    }
                }
            }
            Err(_) => continue,
        }
    }

    let mut rows: Vec<ActionRow> = by_txid
        .values()
        .filter_map(|tx| classify_tx(tx, &our))
        .collect();

    // Newest first: unconfirmed (height=None) leads, then confirmed by height
    // desc, ties broken by txid for a stable order.
    rows.sort_by(|a, b| match (a.height, b.height) {
        (None, None) => a.txid.cmp(&b.txid),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(x), Some(y)) => y.cmp(&x).then_with(|| a.txid.cmp(&b.txid)),
    });

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn addrs(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn classify_receive_from_external() {
        let ours = addrs(&["hs1qmine"]);
        let tx = json!({
            "hash": "aa",
            "height": 100,
            "time": 1000,
            "inputs": [
                {"prevout": {"hash": "pp", "index": 0},
                 "coin": {"value": 500_000_000, "address": "hs1qother",
                          "covenant": {"type": 0, "items": []}}}
            ],
            "outputs": [
                {"value": 100_000_000, "address": "hs1qmine",
                 "covenant": {"type": 0, "action": "NONE", "items": []}}
            ]
        });
        let row = classify_tx(&tx, &ours).unwrap();
        assert_eq!(row.action, "receive");
        assert_eq!(row.direction, "receive");
        assert_eq!(row.value_doos, 100_000_000);
        assert_eq!(row.height, Some(100));
        assert!(row.confirmed);
        assert!(row.name.is_none() && row.name_hash.is_none());
    }

    #[test]
    fn classify_send_to_external() {
        let ours = addrs(&["hs1qmine"]);
        let tx = json!({
            "hash": "bb",
            "height": 101,
            "inputs": [
                {"prevout": {"hash": "pp", "index": 0},
                 "coin": {"value": 400_000_000, "address": "hs1qmine",
                          "covenant": {"type": 0, "items": []}}}
            ],
            "outputs": [
                {"value": 250_000_000, "address": "hs1qdest",
                 "covenant": {"type": 0, "action": "NONE", "items": []}},
                {"value": 149_000_000, "address": "hs1qmine",
                 "covenant": {"type": 0, "action": "NONE", "items": []}}
            ]
        });
        let row = classify_tx(&tx, &ours).unwrap();
        assert_eq!(row.action, "send");
        assert_eq!(row.direction, "send");
        assert_eq!(row.value_doos, -250_000_000);
        assert_eq!(row.counterparty.as_deref(), Some("hs1qdest"));
    }

    #[test]
    fn classify_bid_carries_raw_name() {
        let ours = addrs(&["hs1qmine"]);
        // items = [nameHash, u32(start), rawName, blind]. rawName is
        // hex-encoded UTF-8. "foo" -> hex 666f6f.
        let tx = json!({
            "hash": "cc",
            "height": 200,
            "inputs": [
                {"prevout": {"hash": "pp", "index": 0},
                 "coin": {"value": 1_000_000_000, "address": "hs1qmine",
                          "covenant": {"type": 0, "items": []}}}
            ],
            "outputs": [
                {"value": 5_000_000, "address": "hs1qmine",
                 "covenant": {"type": 3, "action": "BID",
                              "items": ["deadbeef", "000000c8", "666f6f", "cafebabe"]}}
            ]
        });
        let row = classify_tx(&tx, &ours).unwrap();
        assert_eq!(row.action, "bid");
        assert_eq!(row.name.as_deref(), Some("foo"));
        assert_eq!(row.name_hash.as_deref(), Some("deadbeef"));
        // Bid output self-homes onto our own address — nothing leaves the
        // wallet beyond fee, so value_doos is 0 (matches `netSpendDoos`).
        assert_eq!(row.value_doos, 0);
        assert_eq!(row.direction, "send");
    }

    #[test]
    fn classify_reveal_no_raw_name() {
        let ours = addrs(&["hs1qmine"]);
        let tx = json!({
            "hash": "dd",
            "height": 210,
            "inputs": [
                {"prevout": {"hash": "pp", "index": 0},
                 "coin": {"value": 5_500_000, "address": "hs1qmine",
                          "covenant": {"type": 3, "items": ["deadbeef"]}}}
            ],
            "outputs": [
                {"value": 5_000_000, "address": "hs1qmine",
                 "covenant": {"type": 4, "action": "REVEAL",
                              "items": ["deadbeef", "0000d2f0", "cafebabe"]}}
            ]
        });
        let row = classify_tx(&tx, &ours).unwrap();
        assert_eq!(row.action, "reveal");
        assert!(row.name.is_none());
        assert_eq!(row.name_hash.as_deref(), Some("deadbeef"));
        // Reveal output re-homes the bid coin back onto our own address.
        assert_eq!(row.value_doos, 0);
    }

    #[test]
    fn classify_transfer_captures_new_owner() {
        let ours = addrs(&["hs1qmine"]);
        let tx = json!({
            "hash": "ee",
            "height": 300,
            "inputs": [
                {"prevout": {"hash": "pp", "index": 0},
                 "coin": {"value": 5_000_000, "address": "hs1qmine",
                          "covenant": {"type": 6, "items": ["deadbeef"]}}}
            ],
            "outputs": [
                {"value": 5_000_000, "address": "hs1qbuyer",
                 "covenant": {"type": 9, "action": "TRANSFER",
                              "items": ["deadbeef", "00", "aabbccddeeff"]}}
            ]
        });
        let row = classify_tx(&tx, &ours).unwrap();
        assert_eq!(row.action, "transfer");
        assert_eq!(row.counterparty.as_deref(), Some("hs1qbuyer"));
        assert_eq!(row.direction, "send");
        // TRANSFER's covenant output lands on an EXTERNAL address (the new
        // owner). The locked name value leaves this wallet, so value_doos is
        // the sent amount, negative.
        assert_eq!(row.value_doos, -5_000_000);
    }

    #[test]
    fn classify_unconfirmed_marks_mempool() {
        let ours = addrs(&["hs1qmine"]);
        let tx = json!({
            "hash": "ff",
            "height": -1,
            "block": null,
            "confirmations": 0,
            "inputs": [
                {"prevout": {"hash": "pp", "index": 0},
                 "coin": {"value": 500_000_000, "address": "hs1qother",
                          "covenant": {"type": 0, "items": []}}}
            ],
            "outputs": [
                {"value": 100_000_000, "address": "hs1qmine",
                 "covenant": {"type": 0, "items": []}}
            ]
        });
        let row = classify_tx(&tx, &ours).unwrap();
        assert!(!row.confirmed);
        assert!(row.height.is_none());
        assert!(row.time.is_none());
    }

    #[test]
    fn classify_ignores_unrelated_tx() {
        let ours = addrs(&["hs1qmine"]);
        let tx = json!({
            "hash": "gg",
            "height": 1,
            "inputs": [
                {"prevout": {"hash": "pp", "index": 0},
                 "coin": {"value": 1, "address": "hs1qstranger",
                          "covenant": {"type": 0, "items": []}}}
            ],
            "outputs": [
                {"value": 1, "address": "hs1qelse",
                 "covenant": {"type": 0, "items": []}}
            ]
        });
        assert!(classify_tx(&tx, &ours).is_none());
    }

    #[test]
    fn classify_register_from_own_reveal() {
        let ours = addrs(&["hs1qmine"]);
        let tx = json!({
            "hash": "hh",
            "height": 400,
            "inputs": [
                {"prevout": {"hash": "pp", "index": 0},
                 "coin": {"value": 5_000_000, "address": "hs1qmine",
                          "covenant": {"type": 4, "items": ["deadbeef"]}}}
            ],
            "outputs": [
                {"value": 5_000_000, "address": "hs1qmine",
                 "covenant": {"type": 6, "action": "REGISTER",
                              "items": ["deadbeef", "0000", "00"]}}
            ]
        });
        let row = classify_tx(&tx, &ours).unwrap();
        assert_eq!(row.action, "register");
        assert_eq!(row.direction, "send");
        assert_eq!(row.name_hash.as_deref(), Some("deadbeef"));
        // REGISTER's covenant output self-homes onto our address — no net flow.
        assert_eq!(row.value_doos, 0);
    }

    #[test]
    fn classify_update_self_homed_is_zero() {
        // Regression for the "0 in wallet vs 100 in history" bug: a DNS
        // UPDATE re-homes the name's locked value (here 100 HNS) back to our
        // own coin. The drafts card reports 0 for the identical tx via
        // `netSpendDoos`; the history classifier must agree.
        let ours = addrs(&["hs1qmine"]);
        let tx = json!({
            "hash": "e059a8c061",
            "height": 500,
            "inputs": [
                {"prevout": {"hash": "pp", "index": 0},
                 "coin": {"value": 100_000_000, "address": "hs1qmine",
                          "covenant": {"type": 6, "items": ["deadbeef"]}}}
            ],
            "outputs": [
                {"value": 100_000_000, "address": "hs1qmine",
                 "covenant": {"type": 7, "action": "UPDATE",
                              "items": ["deadbeef", "0000", "00"]}}
            ]
        });
        let row = classify_tx(&tx, &ours).unwrap();
        assert_eq!(row.action, "update");
        assert_eq!(row.direction, "send");
        assert_eq!(row.value_doos, 0);
        assert_eq!(row.name_hash.as_deref(), Some("deadbeef"));
    }
}
