use crate::noncustodial::network::Network;
use crate::noncustodial::shakedex::listing_file::{ListingFile, MAX_LISTING_FILE_BYTES};
use serde_json::Value;

const DEX: &str = include_str!("../../tests/vectors/shakedex/proof_dexreviews.json");
const COOL: &str = include_str!("../../tests/vectors/shakedex/proof_cooljobs.json");
fn v(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

#[test]
fn parses_live_listing() {
    let l = ListingFile::parse(DEX, Network::Main).unwrap();
    assert_eq!(l.name, "dexreviews");
    assert_eq!(l.steps.len(), 1);
    assert_eq!(l.steps[0].price, 435_000_000);
    assert_eq!(l.expires_at, Some(1_815_232_480));
}

#[test]
fn round_trips_unchanged() {
    for f in [DEX, COOL] {
        let l = ListingFile::parse(f, Network::Main).unwrap();
        assert_eq!(v(&l.to_json().unwrap()), v(f));
    }
}

/// Fields the parser fills in or normalises come back exactly as the file
/// wrote them: absent stays absent, null stays null, hex keeps its case.
#[test]
fn round_trip_keeps_absent_null_and_hex_case_as_written() {
    let mut j = v(DEX);
    let obj = j.as_object_mut().unwrap();
    obj.remove("feeAddr");
    obj.insert("expiresAt".into(), Value::Null);
    let upper = obj["lockingTxHash"].as_str().unwrap().to_uppercase();
    obj.insert("lockingTxHash".into(), upper.into());
    let step = j["data"][0].as_object_mut().unwrap();
    step.remove("fee");
    let sig = step["signature"].as_str().unwrap().to_uppercase();
    step.insert("signature".into(), sig.into());

    let l = ListingFile::parse(&j.to_string(), Network::Main).unwrap();
    assert_eq!(v(&l.to_json().unwrap()), j);
}

#[test]
fn unknown_fields_preserved() {
    let mut j = v(DEX);
    j["marketNote"] = "hello".into();
    let l = ListingFile::parse(&j.to_string(), Network::Main).unwrap();
    assert_eq!(v(&l.to_json().unwrap())["marketNote"], "hello");
}

#[test]
fn only_version_two() {
    for ver in [1u64, 3] {
        let mut j = v(DEX);
        j["version"] = ver.into();
        assert!(ListingFile::parse(&j.to_string(), Network::Main).is_err());
    }
    assert!(ListingFile::parse("SHAKEDEX_PROOF:1.0.0\n", Network::Main).is_err());
}

#[test]
fn wrong_sighash_byte_refused() {
    let mut j = v(DEX);
    let sig = j["data"][0]["signature"].as_str().unwrap().to_owned();
    j["data"][0]["signature"] = format!("{}01", &sig[..128]).into();
    assert!(ListingFile::parse(&j.to_string(), Network::Main).is_err());
}

/// secp256k1's group order: `s` and `n - s` sign the same message.
const ORDER: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe,
    0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41,
];

/// The same signature with `s` replaced by `n - s`: still valid, but high-S.
fn high_s(sig_hex: &str) -> String {
    let mut sig = hex::decode(sig_hex).unwrap();
    let mut borrow = 0i16;
    for i in (0..32).rev() {
        let d = ORDER[i] as i16 - sig[32 + i] as i16 - borrow;
        borrow = (d < 0) as i16;
        sig[32 + i] = d.rem_euclid(256) as u8;
    }
    hex::encode(sig)
}

#[test]
fn high_s_signature_refused_at_import() {
    let mut j = v(DEX);
    let sig = j["data"][0]["signature"].as_str().unwrap().to_owned();
    let flipped = high_s(&sig);
    assert_ne!(flipped, sig);
    assert!(flipped.ends_with("84"), "the sighash byte is untouched");
    j["data"][0]["signature"] = flipped.into();
    let err = ListingFile::parse(&j.to_string(), Network::Main).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Invalid input: listing file: price step signature is not low-S"
    );
}

#[test]
fn fee_addr_with_zero_fee_means_no_fee_output() {
    let mut j = v(DEX);
    j["feeAddr"] = j["paymentAddr"].clone();
    let l = ListingFile::parse(&j.to_string(), Network::Main).unwrap();
    assert!(l.fee_addr.is_none());
    assert!(l.fee_output_address(Network::Main).is_none());
    // R2: not interpreted, but preserved on round-trip.
    assert_eq!(v(&l.to_json().unwrap())["feeAddr"], j["feeAddr"]);
}

#[test]
fn unknown_step_fields_preserved() {
    let mut j = v(DEX);
    j["data"][0]["stepNote"] = "hello".into();
    let l = ListingFile::parse(&j.to_string(), Network::Main).unwrap();
    assert_eq!(v(&l.to_json().unwrap()), j);
}

#[test]
fn other_network_prefix_refused() {
    assert!(ListingFile::parse(DEX, Network::Regtest).is_err());
}

#[test]
fn oversized_and_empty_refused() {
    let padded = |total: usize| {
        let mut s = v(DEX).to_string();
        assert!(s.len() <= total);
        s.push_str(&" ".repeat(total - s.len()));
        s
    };
    let err = ListingFile::parse(&padded(MAX_LISTING_FILE_BYTES + 1), Network::Main).unwrap_err();
    assert!(err.to_string().contains("larger than"), "{err}");
    assert!(ListingFile::parse(&padded(MAX_LISTING_FILE_BYTES), Network::Main).is_ok());
    let mut j = v(DEX);
    j["data"] = Value::Array(vec![]);
    assert!(ListingFile::parse(&j.to_string(), Network::Main).is_err());
}

#[test]
fn malformed_fields_refused_without_panic() {
    for (k, bad) in [
        ("lockingTxHash", "zz"),
        ("publicKey", "03ab"),
        ("paymentAddr", "hs1nope"),
    ] {
        let mut j = v(DEX);
        j[k] = bad.into();
        assert!(
            ListingFile::parse(&j.to_string(), Network::Main).is_err(),
            "{k}"
        );
    }
}

fn with_fee(fee_addr: &str) -> String {
    let mut j = v(DEX);
    j["data"][0]["fee"] = 1000u64.into();
    j["feeAddr"] = fee_addr.into();
    j.to_string()
}

#[test]
fn unusable_fee_addr_means_no_fee_output() {
    // An unusable feeAddr does not refuse the listing: the seller's
    // signature does not cover it, so it only means no fee output is paid.
    use crate::noncustodial::address;
    let h20 = [7u8; 20];
    let other_net = address::encode_p2wpkh(Network::Regtest, &h20).unwrap();
    let wsh = address::encode_p2wsh(Network::Main, &[9u8; 32]).unwrap();
    for bad in [other_net.as_str(), wsh.as_str(), "hs1nope", ""] {
        let l = ListingFile::parse(&with_fee(bad), Network::Main)
            .unwrap_or_else(|e| panic!("{bad:?}: {e}"));
        assert!(l.fee_output_address(Network::Main).is_none(), "{bad:?}");
    }

    let ok = address::encode_p2wpkh(Network::Main, &h20).unwrap();
    let l = ListingFile::parse(&with_fee(&ok), Network::Main).unwrap();
    assert_eq!(l.fee_addr.as_deref(), Some(ok.as_str()));
    assert!(l.fee_output_address(Network::Main).is_some());
}

/// A name hsd would refuse (upper case, a space, empty, too long) is refused
/// when the file is read, not later as "could not be checked".
#[test]
fn invalid_name_refused_at_import() {
    let long = "a".repeat(64);
    for bad in ["DexReviews", "dex reviews", "", "-dex", long.as_str()] {
        let mut j = v(DEX);
        j["name"] = bad.into();
        let err = ListingFile::parse(&j.to_string(), Network::Main).unwrap_err();
        assert!(err.to_string().contains("name"), "{bad:?}: {err}");
    }
}

/// A step whose lock time cannot be encoded (R3: 40 bits of seconds) can
/// never become valid, and a price or fee above the money supply can never
/// be paid: both are refused when the file is read, not shown as a listing
/// that "failed verification" or offered with a Buy that cannot work.
#[test]
fn unencodable_lock_time_or_amount_above_money_supply_refused_at_import() {
    use crate::noncustodial::shakedex::purchase::MAX_MONEY;
    let cases: [(&str, &str, Value); 4] = [
        ("lock time", "lockTime", (1u64 << 40).into()),
        ("price", "price", (MAX_MONEY + 1).into()),
        ("fee", "fee", (MAX_MONEY + 1).into()),
        (
            "price plus fee",
            "fee",
            (MAX_MONEY - 435_000_000 + 1).into(),
        ),
    ];
    for (case, field, value) in cases {
        let mut j = v(DEX);
        j["data"][0][field] = value;
        if field == "fee" {
            j["feeAddr"] = j["paymentAddr"].clone();
        }
        assert!(
            ListingFile::parse(&j.to_string(), Network::Main).is_err(),
            "{case}"
        );
    }
    // The bounds themselves are readable.
    let mut j = v(DEX);
    j["data"][0]["lockTime"] = ((1u64 << 40) - 1).into();
    j["data"][0]["price"] = MAX_MONEY.into();
    assert!(ListingFile::parse(&j.to_string(), Network::Main).is_ok());
}

/// The Market feed (`GET /api/v2/auctions`, fetched live 2026-10-07) writes
/// `expiresAt` as a naive UTC ISO-8601 string; the proof.json of the same
/// dexreviews listing writes it as 1815232480 seconds.
const FEED_PAGE: &str = include_str!("../../tests/vectors/shakedex/market_auctions_page.json");

#[test]
fn live_market_feed_rows_parse_with_their_iso_expiry() {
    let page = v(FEED_PAGE);
    let rows = page["auctions"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        let l = ListingFile::parse(&row.to_string(), Network::Main)
            .unwrap_or_else(|e| panic!("{}: {e}", row["name"]));
        assert_eq!(l.expires_at, Some(1_815_232_480), "{}", row["name"]);
        assert_eq!(v(&l.to_json().unwrap()), *row, "{}", row["name"]);
    }
}

#[test]
fn iso_expiry_with_fractional_seconds_reads_the_whole_second() {
    let mut j = v(DEX);
    j["expiresAt"] = "2027-07-10T15:14:40.808210".into();
    let l = ListingFile::parse(&j.to_string(), Network::Main).unwrap();
    assert_eq!(l.expires_at, Some(1_815_232_480));
}

#[test]
fn unreadable_expiry_refused() {
    for bad in [
        Value::from("next year"),
        Value::from("2027-07-10"),
        Value::from("1969-12-31T23:59:59"),
        Value::from(-1),
        Value::from(1.5),
        Value::Bool(true),
    ] {
        let mut j = v(DEX);
        j["expiresAt"] = bad.clone();
        assert!(
            ListingFile::parse(&j.to_string(), Network::Main).is_err(),
            "{bad}"
        );
    }
}
