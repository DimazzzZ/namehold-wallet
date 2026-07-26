//! Parser for the Namebase account-history CSV export (and the live API, which
//! returns the same CSV).
//!
//! The export is a one-shot historical artifact: Namebase stopped recording
//! account activity on 2026-06-12. Its unique, unreconstructable value is the
//! off-chain data the app cannot derive from the hsd node:
//!   - Namebase platform fees per action (feeCharged / prepaidFee / hnsFee /
//!     totalFee).
//!   - USD proceeds from subdomain / marketplace sales.
//!   - Bid vs. stake amounts at bid time (hidden on-chain until REVEAL).
//!   - Stable Namebase correlation IDs (auctionId / bidId / saleId).
//!
//! CSV shape (verified against a real export):
//!   - 3 preamble comment lines + 1 blank line, then the header
//!     `id,created_at,type,data`.
//!   - `type` is a namespaced string like `auctions:place-bid:4`; the family is
//!     the part before the first `:`, the verb is the middle, and the trailing
//!     `:N` is a schema version we ignore for classification.
//!   - `data` is a CSV-quoted JSON object whose shape depends on `type`. Money
//!     fields are STRINGS in base units (HNS = dollarydoos, USD = cents).
//!
//! The parser is pure (no DB / no network) so it can be unit-tested against
//! inline fixture rows — one per event family.

use serde::Serialize;
use serde_json::Value;

use crate::error::AppError;

/// One classified row of Namebase account history. `camelCase` on the wire to
/// match the frontend convention used by the other Namebase/read types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamebaseEvent {
    /// Namebase event id — globally unique, our idempotency key.
    pub id: i64,
    /// ISO-8601 UTC timestamp from the export.
    pub created_at: String,
    /// Raw event type, e.g. `auctions:place-bid:4`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Family: substring before the first `:` (e.g. `auctions`).
    pub family: String,
    /// Verb: the middle segment (e.g. `place-bid`).
    pub verb: String,
    /// Normalized domain name (lowercased, leading dot stripped), when the
    /// event carries one. `None` for non-name events (deposits, exchange).
    pub name: Option<String>,
    /// Namebase platform fee in dollarydoos, whichever fee field applies.
    pub fee_doos: Option<i64>,
    /// Bid amount in dollarydoos.
    pub bid_doos: Option<i64>,
    /// Stake amount in dollarydoos.
    pub stake_doos: Option<i64>,
    /// Sale proceeds in USD cents (subdomain / marketplace sales).
    pub usd_cents: Option<i64>,
    /// HNS amount in dollarydoos (delivered HNS / deposit amount).
    pub hns_doos: Option<i64>,
    /// Stable Namebase auction id, when present.
    pub auction_id: Option<String>,
    /// Stable Namebase bid id, when present.
    pub bid_id: Option<String>,
    /// Stable Namebase sale id, when present.
    pub sale_id: Option<String>,
    /// Full original `data` JSON, serialized as a compact string. Nothing is
    /// dropped so the UI can show fields we didn't promote to a column.
    pub data_json: String,
}

/// Split a raw `type` like `auctions:place-bid:4` into `(family, verb)`.
/// The trailing `:N` schema version (if any) is dropped from the verb.
fn split_type(kind: &str) -> (String, String) {
    let mut parts = kind.splitn(2, ':');
    let family = parts.next().unwrap_or("").to_string();
    let rest = parts.next().unwrap_or("");
    // `rest` is `place-bid:4` -> verb is everything before the last `:` when
    // that last segment is a pure integer version; otherwise the whole rest.
    let verb = match rest.rsplit_once(':') {
        Some((head, tail)) if tail.chars().all(|c| c.is_ascii_digit()) && !tail.is_empty() => {
            head.to_string()
        }
        _ => rest.to_string(),
    };
    (family, verb)
}

/// Normalize a domain name: trim, strip a single leading dot, lowercase.
fn normalize_name(raw: &str) -> Option<String> {
    let n = raw.trim().trim_start_matches('.').trim().to_lowercase();
    if n.is_empty() {
        None
    } else {
        Some(n)
    }
}

/// Read a base-unit string field (e.g. `"123000000"`) as i64 dollarydoos.
/// Namebase encodes all HNS amounts as integer strings. Negative values are
/// allowed (e.g. `misc:admin-gift` can be negative).
fn doos(data: &Value, key: &str) -> Option<i64> {
    data.get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| s.trim().parse::<i64>().ok())
}

/// Read a nested USD money object like
/// `{"amountString":"2900","asset":"USD"}` as i64 cents. Only returns a value
/// when the asset is USD (guards against reading an HNS object as cents).
fn usd_cents(data: &Value, key: &str) -> Option<i64> {
    let obj = data.get(key)?;
    let asset = obj.get("asset").and_then(|v| v.as_str()).unwrap_or("");
    if asset != "USD" {
        return None;
    }
    obj.get("amountString")
        .and_then(|v| v.as_str())
        .and_then(|s| s.trim().parse::<i64>().ok())
}

/// Read a nested HNS money object like
/// `{"amountString":"4832721250","asset":"HNS"}` as i64 dollarydoos.
fn hns_from_obj(data: &Value, key: &str) -> Option<i64> {
    let obj = data.get(key)?;
    let asset = obj.get("asset").and_then(|v| v.as_str()).unwrap_or("");
    if asset != "HNS" {
        return None;
    }
    obj.get("amountString")
        .and_then(|v| v.as_str())
        .and_then(|s| s.trim().parse::<i64>().ok())
}

/// Pick the first present fee field. Namebase uses different keys per event
/// type, but at most one applies per event.
fn any_fee(data: &Value) -> Option<i64> {
    doos(data, "feeChargedString")
        .or_else(|| doos(data, "prepaidFeeString"))
        .or_else(|| doos(data, "hnsFeeAmountString"))
        .or_else(|| doos(data, "totalFeeString"))
}

/// Build a [`NamebaseEvent`] from the raw CSV columns. Pure; unknown event
/// types still produce a row (promoted columns NULL, full JSON preserved).
fn build_event(id: i64, created_at: &str, kind: &str, data: &Value) -> NamebaseEvent {
    let (family, verb) = split_type(kind);

    // Name lives under `domainName` (auctions/marketplace) or `domain`
    // (subdomains). For subdomain events that also carry a `subdomain`
    // label, the real identity is `{subdomain}.{domain}` — anything else
    // loses the subdomain component and merges rows that aren't actually
    // about the same name.
    let name = if let Some(dn) = data.get("domainName").and_then(|v| v.as_str()) {
        normalize_name(dn)
    } else if let Some(dom) = data.get("domain").and_then(|v| v.as_str()) {
        match data.get("subdomain").and_then(|v| v.as_str()) {
            Some(sub) if !sub.trim().is_empty() => {
                normalize_name(&format!("{}.{}", sub, dom))
            }
            _ => normalize_name(dom),
        }
    } else {
        None
    };

    // USD sale proceeds: `deliveredAmountUsd` (confirm-transfer) or
    // `deliveredAmount`/`receivedAmount` (initialize-transfer, all USD objects).
    let usd = usd_cents(data, "deliveredAmountUsd")
        .or_else(|| usd_cents(data, "deliveredAmount"))
        .or_else(|| usd_cents(data, "receivedAmount"));

    // HNS delivered/deposited: nested `deliveredAmountHns` object, or a
    // top-level `amountString` for deposits/gifts/exchange.
    let hns = hns_from_obj(data, "deliveredAmountHns").or_else(|| doos(data, "amountString"));

    NamebaseEvent {
        id,
        created_at: created_at.to_string(),
        kind: kind.to_string(),
        family,
        verb,
        name,
        fee_doos: any_fee(data),
        bid_doos: doos(data, "bidAmountString"),
        stake_doos: doos(data, "stakeAmountString"),
        usd_cents: usd,
        hns_doos: hns,
        auction_id: data
            .get("auctionId")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        bid_id: data
            .get("bidId")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        sale_id: data
            .get("saleId")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        data_json: data.to_string(),
    }
}

/// Return the substring of `csv_text` starting at the real header line
/// (`id,created_at,type,data`), skipping the human-readable preamble comment
/// lines and blank lines the export prepends. Returns `None` if no header is
/// found.
fn slice_from_header(csv_text: &str) -> Option<&str> {
    let mut offset = 0usize;
    for line in csv_text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "id,created_at,type,data" {
            return Some(&csv_text[offset..]);
        }
        offset += line.len();
    }
    None
}

/// Parse the full Namebase account-history CSV into classified events.
///
/// Skips the preamble, then reads `id,created_at,type,data` records. A row whose
/// `data` isn't valid JSON, or whose `id` isn't an integer, is skipped (the
/// export is machine-generated, so this is defensive rather than expected).
pub fn parse_history_csv(csv_text: &str) -> Result<Vec<NamebaseEvent>, AppError> {
    let body = slice_from_header(csv_text).ok_or_else(|| {
        AppError::InvalidInput(
            "not a Namebase account-history export (missing 'id,created_at,type,data' header)"
                .to_string(),
        )
    })?;

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .trim(csv::Trim::None)
        .from_reader(body.as_bytes());

    let mut events = Vec::new();
    for record in rdr.records() {
        let record = match record {
            Ok(r) => r,
            Err(_) => continue, // skip malformed physical row
        };
        let id = match record.get(0).and_then(|s| s.trim().parse::<i64>().ok()) {
            Some(v) => v,
            None => continue,
        };
        let created_at = record.get(1).unwrap_or("").trim();
        let kind = record.get(2).unwrap_or("").trim();
        let data_raw = record.get(3).unwrap_or("");
        let data: Value = match serde_json::from_str(data_raw) {
            Ok(v) => v,
            Err(_) => Value::Object(serde_json::Map::new()),
        };
        events.push(build_event(id, created_at, kind, &data));
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the full CSV (preamble + header + rows) the way a real export
    /// looks, so tests exercise the preamble-skipping path too.
    fn wrap(rows: &str) -> String {
        format!(
            "\"This export covers your Namebase account history only.\"\n\"\"\n\"It does NOT include Sunset activity.\"\n\nid,created_at,type,data\n{}",
            rows
        )
    }

    #[test]
    fn split_type_strips_version() {
        assert_eq!(
            split_type("auctions:place-bid:4"),
            ("auctions".to_string(), "place-bid".to_string())
        );
        assert_eq!(
            split_type("subdomains:confirm-transfer:2"),
            ("subdomains".to_string(), "confirm-transfer".to_string())
        );
        // No trailing version.
        assert_eq!(
            split_type("misc:admin-gift"),
            ("misc".to_string(), "admin-gift".to_string())
        );
    }

    #[test]
    fn parses_place_bid_with_bid_stake_and_fee() {
        let csv = wrap(
            "188679284,2026-01-17T12:37:54.492Z,auctions:place-bid:4,\"{\"\"domainName\"\":\"\"diver\"\",\"\"auctionId\"\":\"\"90e52360-0b95-4a1b-84f2-8ca2ba697e47\"\",\"\"custodian\"\":\"\"uk\"\",\"\"bidAmountString\"\":\"\"123000000\"\",\"\"stakeAmountString\"\":\"\"2469000000\"\",\"\"prepaidFeeString\"\":\"\"1000283\"\"}\"\n",
        );
        let events = parse_history_csv(&csv).unwrap();
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.id, 188679284);
        assert_eq!(e.family, "auctions");
        assert_eq!(e.verb, "place-bid");
        assert_eq!(e.name.as_deref(), Some("diver"));
        assert_eq!(e.bid_doos, Some(123000000));
        assert_eq!(e.stake_doos, Some(2469000000));
        assert_eq!(e.fee_doos, Some(1000283)); // prepaidFeeString
        assert_eq!(
            e.auction_id.as_deref(),
            Some("90e52360-0b95-4a1b-84f2-8ca2ba697e47")
        );
    }

    #[test]
    fn parses_confirm_transfer_usd_and_hns() {
        let csv = wrap(
            "188784786,2026-01-27T06:25:25.161Z,subdomains:confirm-transfer:2,\"{\"\"domain\"\":\"\"shot\"\",\"\"subdomain\"\":\"\"moon\"\",\"\"saleId\"\":\"\"37e1df8c-07e7-4f73-a2b5-75330c2a10f2\"\",\"\"deliveredAmountUsd\"\":{\"\"amountString\"\":\"\"2900\"\",\"\"asset\"\":\"\"USD\"\"},\"\"deliveredAmountHns\"\":{\"\"amountString\"\":\"\"4832721250\"\",\"\"asset\"\":\"\"HNS\"\"}}\"\n",
        );
        let events = parse_history_csv(&csv).unwrap();
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.family, "subdomains");
        assert_eq!(e.verb, "confirm-transfer");
        assert_eq!(e.name.as_deref(), Some("moon.shot"));
        assert_eq!(e.usd_cents, Some(2900));
        assert_eq!(e.hns_doos, Some(4832721250));
        assert_eq!(e.sale_id.as_deref(), Some("37e1df8c-07e7-4f73-a2b5-75330c2a10f2"));
        // No bid/stake on a sale.
        assert_eq!(e.bid_doos, None);
        assert_eq!(e.stake_doos, None);
    }

    #[test]
    fn parses_charge_fee() {
        let csv = wrap(
            "188680273,2026-01-17T18:32:34.289Z,auctions:charge-fee:0,\"{\"\"domainName\"\":\"\"diver\"\",\"\"auctionId\"\":\"\"90e52360\"\",\"\"covenant\"\":\"\"BID\"\",\"\"custodian\"\":\"\"uk\"\",\"\"feeChargedString\"\":\"\"32600\"\"}\"\n",
        );
        let e = &parse_history_csv(&csv).unwrap()[0];
        assert_eq!(e.verb, "charge-fee");
        assert_eq!(e.fee_doos, Some(32600));
        assert_eq!(e.name.as_deref(), Some("diver"));
    }

    #[test]
    fn parses_renewal_fee_hns_key() {
        let csv = wrap(
            "189698552,2026-06-03T07:23:33.653Z,auctions:charge-renewal-fee:0,\"{\"\"custodian\"\":\"\"uk\"\",\"\"domainName\"\":\"\"dimmer\"\",\"\"hnsFeeAmountString\"\":\"\"10000000\"\"}\"\n",
        );
        let e = &parse_history_csv(&csv).unwrap()[0];
        assert_eq!(e.verb, "charge-renewal-fee");
        assert_eq!(e.fee_doos, Some(10000000)); // hnsFeeAmountString
    }

    #[test]
    fn parses_deposit_amount_and_negative_gift() {
        let csv = wrap(
            "24632896,2020-09-22T10:03:36.467Z,wallet:deposit:1,\"{\"\"asset\"\":\"\"BTC\"\",\"\"amountString\"\":\"\"1097674\"\",\"\"txHash\"\":\"\"abc\"\"}\"\n33287041,2021-04-16T00:59:20.126Z,misc:admin-gift:1,\"{\"\"asset\"\":\"\"HNS\"\",\"\"amountString\"\":\"\"-100000000\"\"}\"\n",
        );
        let events = parse_history_csv(&csv).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].family, "wallet");
        assert_eq!(events[0].hns_doos, Some(1097674));
        assert_eq!(events[0].name, None);
        assert_eq!(events[1].verb, "admin-gift");
        assert_eq!(events[1].hns_doos, Some(-100000000)); // negative allowed
    }

    #[test]
    fn unknown_type_still_produces_row_with_full_json() {
        let csv = wrap(
            "999,2026-01-01T00:00:00.000Z,some:brand-new-event:9,\"{\"\"weird\"\":\"\"payload\"\",\"\"domainName\"\":\"\"foo\"\"}\"\n",
        );
        let e = &parse_history_csv(&csv).unwrap()[0];
        assert_eq!(e.family, "some");
        assert_eq!(e.verb, "brand-new-event");
        assert_eq!(e.name.as_deref(), Some("foo"));
        assert_eq!(e.fee_doos, None);
        assert!(e.data_json.contains("weird"));
    }

    #[test]
    fn missing_header_is_an_error() {
        let err = parse_history_csv("some,other,csv\n1,2,3\n").unwrap_err();
        match err {
            AppError::InvalidInput(_) => {}
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn name_is_lowercased_and_dot_stripped() {
        assert_eq!(normalize_name(".Diver"), Some("diver".to_string()));
        assert_eq!(normalize_name("  "), None);
    }

    #[test]
    fn parses_initialize_transfer_composes_subdomain_name() {
        let csv = wrap(
            "188784785,2026-01-27T06:25:19.730Z,subdomains:initialize-transfer:2,\"{\"\"domain\"\":\"\"shot\"\",\"\"subdomain\"\":\"\"moon\"\",\"\"saleId\"\":\"\"37e1df8c\"\",\"\"receivedAmount\"\":{\"\"amountString\"\":\"\"3000\"\",\"\"asset\"\":\"\"USD\"\"},\"\"deliveredAmount\"\":{\"\"amountString\"\":\"\"2900\"\",\"\"asset\"\":\"\"USD\"\"}}\"\n",
        );
        let e = &parse_history_csv(&csv).unwrap()[0];
        assert_eq!(e.family, "subdomains");
        assert_eq!(e.verb, "initialize-transfer");
        // Full subdomain identity, not just the parent TLD.
        assert_eq!(e.name.as_deref(), Some("moon.shot"));
        // `receivedAmount` is the USD proceeds fallback.
        assert_eq!(e.usd_cents, Some(2900));
    }

    #[test]
    fn stake_domain_uses_domain_only_when_no_subdomain() {
        // `subdomains:stake-domain` carries only `domain` (the whole TLD is
        // staked for subdomains) — there is no subdomain component to compose.
        let csv = wrap(
            "127574416,2023-08-18T15:45:29.209Z,subdomains:stake-domain:0,\"{\"\"domain\"\":\"\"ecology\"\",\"\"custodian\"\":\"\"uk\"\"}\"\n",
        );
        let e = &parse_history_csv(&csv).unwrap()[0];
        assert_eq!(e.family, "subdomains");
        assert_eq!(e.verb, "stake-domain");
        assert_eq!(e.name.as_deref(), Some("ecology"));
    }
}
