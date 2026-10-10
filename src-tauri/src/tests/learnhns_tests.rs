use crate::market::learnhns::{
    market_fee_is_published, name_from_listing_link, FeeInfo, LearnHnsClient, ListingKind,
    MarketReply, PendingListing, ProofCopy, StatusRecorded, StatusReport, UploadAccepted,
    MARKET_MAINNET_ONLY,
};
use crate::noncustodial::network::Network;
use crate::tests::shakedex_cmd_tests::LISTING_FILE;

#[tokio::test]
async fn lists_available_and_fetches_listing_file() {
    let mut s = mockito::Server::new_async().await;
    let _a = s
        .mock(
            "GET",
            "/api/v2/auctions?availability=available&page=1&per_page=100",
        )
        .with_body(r#"{"auctions":[{"name":"dexreviews"}],"availability":"available","total":1}"#)
        .create_async()
        .await;
    let _p = s
        .mock("GET", "/listing/dexreviews/proof.json")
        .with_body("{\"version\":2}")
        .create_async()
        .await;
    let _n = s
        .mock("GET", "/listing/missing/proof.json")
        .with_status(404)
        .create_async()
        .await;
    let c = LearnHnsClient::with_base_url(&s.url()).unwrap();
    let (rows, total) = c.list_available(1, 100).await.unwrap();
    assert_eq!((rows.len(), total), (1, 1));
    assert_eq!(
        c.listing_file("dexreviews").await.unwrap().as_deref(),
        Some("{\"version\":2}")
    );
    assert!(c.listing_file("missing").await.unwrap().is_none());
}

#[tokio::test]
async fn non_2xx_is_an_error_with_status() {
    let mut s = mockito::Server::new_async().await;
    let _f = s
        .mock("GET", "/api/v2/fee_info")
        .with_status(500)
        .create_async()
        .await;
    let c = LearnHnsClient::with_base_url(&s.url()).unwrap();
    let err = c.fee_info().await.unwrap_err().to_string();
    assert!(err.contains("500"), "{err}");
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let mut s = mockito::Server::new_async().await;
    let target = s
        .mock("GET", "/elsewhere.json")
        .with_body("{\"version\":2}")
        .expect(0)
        .create_async()
        .await;
    let _r = s
        .mock("GET", "/listing/dexreviews/proof.json")
        .with_status(302)
        .with_header("location", &format!("{}/elsewhere.json", s.url()))
        .create_async()
        .await;
    let c = LearnHnsClient::with_base_url(&s.url()).unwrap();
    let err = c.listing_file("dexreviews").await.unwrap_err().to_string();
    assert!(err.contains("302"), "{err}");
    target.assert_async().await;
}

#[tokio::test]
async fn oversized_listing_file_is_refused() {
    use crate::noncustodial::shakedex::listing_file::MAX_LISTING_FILE_BYTES;
    let mut s = mockito::Server::new_async().await;
    let _p = s
        .mock("GET", "/listing/big/proof.json")
        .with_body(" ".repeat(MAX_LISTING_FILE_BYTES + 1))
        .create_async()
        .await;
    let _q = s
        .mock("GET", "/listing/fits/proof.json")
        .with_body(" ".repeat(MAX_LISTING_FILE_BYTES))
        .create_async()
        .await;
    let c = LearnHnsClient::with_base_url(&s.url()).unwrap();
    let err = c.listing_file("big").await.unwrap_err().to_string();
    assert!(err.contains("larger than"), "{err}");
    assert_eq!(
        c.listing_file("fits").await.unwrap().map(|t| t.len()),
        Some(MAX_LISTING_FILE_BYTES)
    );
}

#[tokio::test]
async fn oversized_market_page_and_fee_info_are_refused() {
    let mut s = mockito::Server::new_async().await;
    let huge = " ".repeat(8 * 1024 * 1024 + 1);
    let _a = s
        .mock(
            "GET",
            "/api/v2/auctions?availability=available&page=1&per_page=100",
        )
        .with_body(&huge)
        .create_async()
        .await;
    let _f = s
        .mock("GET", "/api/v2/fee_info")
        .with_body(&huge)
        .create_async()
        .await;
    let c = LearnHnsClient::with_base_url(&s.url()).unwrap();
    let err = c.list_available(1, 100).await.unwrap_err().to_string();
    assert!(err.contains("larger than"), "{err}");
    let err = c.fee_info().await.unwrap_err().to_string();
    assert!(err.contains("larger than"), "{err}");
}

#[tokio::test]
async fn reads_fee_info() {
    let mut s = mockito::Server::new_async().await;
    let _f = s
        .mock("GET", "/api/v2/fee_info")
        .with_body(r#"{"addr":null,"address":null,"rate":0}"#)
        .create_async()
        .await;
    let c = LearnHnsClient::with_base_url(&s.url()).unwrap();
    let f = c.fee_info().await.unwrap();
    assert_eq!((f.rate, f.address), (0, None));
}

#[test]
fn base_url_validation() {
    assert!(LearnHnsClient::with_base_url("https://market.learnhns.com").is_ok());
    assert!(LearnHnsClient::with_base_url("http://market.learnhns.com").is_err());
    assert!(LearnHnsClient::with_base_url("https://evil.example").is_err());
    assert!(LearnHnsClient::with_base_url("not a url").is_err());
    assert!(LearnHnsClient::with_base_url("http://127.0.0.1:1234").is_ok());
}

#[test]
fn only_learnhns_listing_links() {
    assert_eq!(
        name_from_listing_link("https://market.learnhns.com/listing/dexreviews").as_deref(),
        Some("dexreviews")
    );
    assert_eq!(
        name_from_listing_link("https://market.learnhns.com/listing/dexreviews/").as_deref(),
        Some("dexreviews")
    );
    assert_eq!(
        name_from_listing_link("https://market.learnhns.com/listing/Dex-1_x").as_deref(),
        Some("dex-1_x")
    );
    // The listing's own file, as the market links it.
    assert_eq!(
        name_from_listing_link("https://market.learnhns.com/listing/enstransfer/proof.json")
            .as_deref(),
        Some("enstransfer")
    );
    assert!(name_from_listing_link("https://market.learnhns.com/listing/x/other.json").is_none());
    // The chain's own name rule: no `-` or `_` at either end.
    for bad in ["-dex", "dex-", "_dex", "dex_"] {
        let link = format!("https://market.learnhns.com/listing/{bad}");
        assert!(name_from_listing_link(&link).is_none(), "{bad}");
    }
    assert!(name_from_listing_link("http://market.learnhns.com/listing/x").is_none());
    assert!(name_from_listing_link("https://evil.example/listing/x").is_none());
    assert!(name_from_listing_link("https://market.learnhns.com/listing/../etc").is_none());
    assert!(name_from_listing_link("https://market.learnhns.com/listing/a/b").is_none());
    assert!(name_from_listing_link("https://market.learnhns.com/listing/x?y=1").is_none());
    assert!(name_from_listing_link("https://market.learnhns.com/listing/x#f").is_none());
    assert!(name_from_listing_link("https://market.learnhns.com/listing/").is_none());
    assert!(name_from_listing_link("https://market.learnhns.com/listing//").is_none());
    let long = format!("https://market.learnhns.com/listing/{}", "a".repeat(64));
    assert!(name_from_listing_link(&long).is_none());
    let ok = format!("https://market.learnhns.com/listing/{}", "a".repeat(63));
    assert!(name_from_listing_link(&ok).is_some());
}

#[test]
fn market_fee_must_match_published() {
    let info = FeeInfo {
        rate: 100,
        address: Some("hs1qfee".into()),
    }; // 1%
    assert!(market_fee_is_published(
        2_500_000,
        Some("hs1qfee"),
        250_000_000,
        &info
    ));
    assert!(!market_fee_is_published(
        2_500_001,
        Some("hs1qfee"),
        250_000_000,
        &info
    ));
    assert!(!market_fee_is_published(
        2_500_000,
        Some("hs1qattacker"),
        250_000_000,
        &info
    ));
    assert!(!market_fee_is_published(
        0,
        Some("hs1qfee"),
        250_000_000,
        &info
    ));
    assert!(!market_fee_is_published(
        2_500_000,
        None,
        250_000_000,
        &info
    ));
    assert!(!market_fee_is_published(
        1,
        None,
        250_000_000,
        &FeeInfo {
            rate: 0,
            address: None
        }
    ));
    // No overflow at the u64 extremes.
    let big = FeeInfo {
        rate: u32::MAX,
        address: Some("a".into()),
    };
    assert!(market_fee_is_published(u64::MAX, Some("a"), u64::MAX, &big));
}

// --- writes (R23, R25, R28): only the market's own JSON answer is a verdict ---
//
// No live reply of a write endpoint is recorded: recording one would publish
// on the real market. The replies below copy the `jsonify` calls of the
// LearnHNS Market source (MKT `shadstone/learnhns-market` @3d117361), field
// for field; the line numbers are in shakedex-protocol.md §11.3.

/// Every request body a mock saw, for asserting on what was sent.
pub(crate) type Seen = std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>;

/// `m` answering `status` with `reply`, recording each request's body.
pub(crate) fn recording(
    m: mockito::Mock,
    status: usize,
    reply: &'static str,
) -> (mockito::Mock, Seen) {
    let seen: Seen = Default::default();
    let s2 = seen.clone();
    let m = m
        .with_status(status)
        .with_header("content-type", "application/json")
        .with_body_from_request(move |req| {
            s2.lock().unwrap().push(req.body().unwrap().clone());
            reply.as_bytes().to_vec()
        });
    (m, seen)
}

/// The file in multipart field `proof` of `body`, and its part headers.
pub(crate) fn proof_part(body: &[u8]) -> (String, String) {
    let text = String::from_utf8(body.to_vec()).unwrap();
    let at = text.find("name=\"proof\"").expect("a proof field");
    let start = text[..at].rfind("\r\n").map_or(0, |i| i + 2);
    let head_end = text[at..].find("\r\n\r\n").unwrap() + at;
    let end = text[head_end + 4..].find("\r\n--").unwrap() + head_end + 4;
    (
        text[start..head_end].to_string(),
        text[head_end + 4..end].to_string(),
    )
}

/// MKT api.py `upload_proof`'s success reply (@3d117361, L2893–2899); no
/// live upload was recorded (it would publish).
pub(crate) const UPLOAD_ACCEPTED: &str = r#"{"cid":"bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku","name":"dexreviews","previousListingId":null,"replaced":false,"success":true}"#;
/// MKT api.py `create_pending_listing`'s reply (@3d117361, L2139–2142); the
/// `pending` object (`_pending_listing_payload`, L353–390) copies the live
/// row recorded 2026-10-03 (`api_v2_pending-listings.json`, `pending[0]`):
/// the same keys and the same `pending-submitted` action words, with this
/// name, outpoint and no price or note.
pub(crate) const PENDING_ACCEPTED: &str = r#"{"pending":{"actionRequired":true,"blocksUntilFinalize":null,"buyable":false,"chainHeight":null,"createdAt":"2026-10-10T05:39:26.279069","expectedPrice":null,"id":"pending-504","listingMode":"fixed-price","lockScriptAddr":"hs1qx3lyaqwuhg7h6ut5rc9m89gna3jqce4ka07nvjmzljkq0696t2gqh449r6","name":"dexreviews","nameState":null,"network":"main","nextAction":"Check the transfer in Bob","nextActionDetail":"The channel has the pending record but cannot yet verify an active Shakedex transfer for this name.","pending":true,"pendingReason":null,"sellerNote":"","status":"pending-submitted","transferHeight":null,"transferOutputIdx":0,"transferTxHash":"abababababababababababababababababababababababababababababababab","updatedAt":"2026-10-10T05:39:26.279074","url":"/listing/dexreviews"},"success":true}"#;
/// MKT `_mark_listing_cancelled_if_spent`'s reply (@3d117361, L873–879).
pub(crate) const CANCEL_RECORDED: &str = r#"{"cancelTxHash":"cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd","cancelled":true,"cancelledAt":"2026-10-10T06:00:00.000000","status":"cancelled","url":"/listing/dexreviews"}"#;
/// MKT `_mark_listing_sold_if_spent`'s reply (@3d117361, L729–737).
pub(crate) const SALE_RECORDED: &str = r#"{"saleTxHash":"b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1","sold":true,"soldAt":"2026-10-10T06:00:00.000000","status":"sold","transferStartTxHash":"b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1","url":"/listing/dexreviews","verificationSource":"hsd"}"#;
/// MKT `refresh_listing_status`'s reply when it has no active or sale-pending
/// listing of the name. We always send a txid, so it comes from
/// `_refreshable_listing_for_spend` (@3d117361, L475–495) with its
/// `last_status = 400` (L478) when the name has no listing to check; the same
/// words come back as 404 on the paths without a txid (L2463/L2475).
pub(crate) const NO_LISTING: &str = r#"{"error":"Listing not found"}"#;
/// MKT `_verified_listing_spend` (@3d117361, L598–611) when neither its hsd
/// (`_fetch_hsd_tx`, L997–1001) nor its explorer (`_fetch_explorer_tx`'s 404,
/// L1004+) has seen the tx yet: chain lag, not a verdict.
pub(crate) const TX_NOT_SEEN: &str = r#"{"error":"Transaction was not found","fallbackError":"Transaction was not found by explorer fallback"}"#;
/// MKT `_verify_listing_proof_on_chain`'s chain-lag replies to an upload
/// (@3d117361): the lock coin (`_fetch_hsd_coin`, L993), the name
/// (`_fetch_hsd_name_info`, L1082), the owner not yet the lock (L1111).
pub(crate) const COIN_NOT_SEEN: &str =
    r#"{"error":"Listing coin was not found or is already spent"}"#;
pub(crate) const NAME_NOT_SEEN: &str = r#"{"error":"Name was not found"}"#;
pub(crate) const OWNER_NOT_SEEN: &str =
    r#"{"error":"The proof locking output is not the name's current on-chain owner coin"}"#;
/// The upload's file name: the listing's name and lock txid, so no two
/// uploads share a name in the market's shared upload folder (MKT api.py
/// L2807–2808 saves under the file name as sent).
const UPLOAD_FILE_NAME: &str =
    "dexreviews-c44db193e4c815969db27870f49805cf6838a3f0ef700165e4d3c3660650abea.json";
/// MKT main.py `listing_proof`'s 404 (@3d117361, L512–513).
pub(crate) const PROOF_NOT_FOUND: &str = r#"{"error":"Active listing not found"}"#;

fn mainnet(s: &mockito::ServerGuard) -> LearnHnsClient {
    LearnHnsClient::with_base_url(&s.url())
        .unwrap()
        .for_network(Network::Main)
}

fn pending() -> PendingListing<'static> {
    PendingListing {
        name: "dexreviews",
        transfer_txid: "abababababababababababababababababababababababababababababababab",
        transfer_vout: 0,
        lock_address: "hs1qx3lyaqwuhg7h6ut5rc9m89gna3jqce4ka07nvjmzljkq0696t2gqh449r6",
        kind: ListingKind::FixedPrice,
    }
}

/// R23, carry 5: every write is refused, before any HTTP, unless the client
/// was told the profile's network and it is mainnet — testnet, regtest,
/// simnet and an unknown network alike.
#[tokio::test]
async fn no_http_off_mainnet() {
    let mut s = mockito::Server::new_async().await;
    let any = s
        .mock("POST", mockito::Matcher::Any)
        .expect(0)
        .create_async()
        .await;
    let cancel = StatusReport::Cancelled {
        cancel_txid: "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
    };
    for net in [
        Some(Network::Testnet),
        Some(Network::Regtest),
        Some(Network::Simnet),
        None,
    ] {
        let base = LearnHnsClient::with_base_url(&s.url()).unwrap();
        let c = match net {
            Some(n) => base.for_network(n),
            None => base,
        };
        let errs = [
            c.upload_proof(LISTING_FILE).await.err(),
            c.post_pending_listing(&pending()).await.err(),
            c.refresh_status("dexreviews", &cancel).await.err(),
        ];
        for e in errs {
            let e = e.expect("refused off mainnet").to_string();
            assert!(e.contains(MARKET_MAINNET_ONLY), "{net:?}: {e}");
        }
    }
    any.assert_async().await;
}

/// R23: the upload is `POST /api/upload-proof`, multipart, the listing file
/// as written in field `proof` (filename `<name>-<lockTxHash>.json`, JSON),
/// and the market's 201 reads as accepted; its own JSON refusal as refused,
/// with its words; its chain-lag replies as not seen yet; a proxy's HTML
/// page, a redirect, a 5xx, an oversized reply or a reply not in its shape
/// as no answer.
#[tokio::test]
async fn upload_shape() {
    let mut s = mockito::Server::new_async().await;
    let (m, seen) = recording(
        s.mock("POST", "/api/upload-proof").match_header(
            "content-type",
            mockito::Matcher::Regex("^multipart/form-data; boundary=".into()),
        ),
        201,
        UPLOAD_ACCEPTED,
    );
    let _m = m.create_async().await;
    let c = mainnet(&s);
    match c.upload_proof(LISTING_FILE).await.unwrap() {
        MarketReply::Accepted(UploadAccepted { name, replaced }) => {
            assert_eq!((name.as_str(), replaced), ("dexreviews", false))
        }
        other => panic!("{other:?}"),
    }
    let bodies = seen.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1);
    let (head, file) = proof_part(&bodies[0]);
    assert!(
        head.contains(&format!(
            r#"form-data; name="proof"; filename="{UPLOAD_FILE_NAME}""#
        )),
        "{head}"
    );
    assert!(
        head.to_ascii_lowercase()
            .contains("content-type: application/json"),
        "{head}"
    );
    assert_eq!(file, LISTING_FILE, "the file as written, byte for byte");

    for (status, body, want) in [
        (
            400,
            r#"{"error":"Fixed-price listings must contain exactly one proof entry"}"#,
            "refused",
        ),
        (
            409,
            r#"{"error":"An active listing for dexreviews already exists. Cancel it or wait for it to expire before uploading a different listing proof."}"#,
            "refused",
        ),
        (
            502,
            "<html><body>Application failed to respond</body></html>",
            "none",
        ),
        (
            429,
            "<!doctype html><title>429 Too Many Requests</title>",
            "none",
        ),
        (500, r#"{"error":"Failed to pin to IPFS: timeout"}"#, "none"),
        (201, r#"{"success":false}"#, "none"),
        (201, "not json", "none"),
        (400, r#"{"message":"bad request"}"#, "none"),
        (201, r#"{"success":true,"replaced":false}"#, "none"),
        (201, r#"{"success":true,"name":"dexreviews"}"#, "none"),
        (404, COIN_NOT_SEEN, "not seen"),
        (404, NAME_NOT_SEEN, "not seen"),
        (409, OWNER_NOT_SEEN, "not seen"),
        // The same words under another status are not the chain-lag reply.
        (400, NAME_NOT_SEEN, "refused"),
    ]
    .map(|(st, b, w)| (st, b.to_string(), w))
    .into_iter()
    // A reply over the 64 KiB cap is not read, whatever it says.
    .chain([(
        400,
        format!(r#"{{"error":"{}"}}"#, "x".repeat(70 * 1024)),
        "none",
    )])
    {
        let mut s = mockito::Server::new_async().await;
        let _m = s
            .mock("POST", "/api/upload-proof")
            .with_status(status)
            .with_body(&body)
            .create_async()
            .await;
        let r = mainnet(&s).upload_proof(LISTING_FILE).await.unwrap();
        match (want, &r) {
            ("refused", MarketReply::Refused { status: got, error }) => {
                assert_eq!(*got as usize, status);
                assert!(body.contains(error.as_str()), "{error}");
            }
            ("not seen", MarketReply::NotSeenYet { status: got, error }) => {
                assert_eq!(*got as usize, status);
                assert!(body.contains(error.as_str()), "{error}");
            }
            ("none", MarketReply::NoAnswer(_)) => {}
            _ => panic!("{status} {body}: {r:?}"),
        }
    }

    // A redirect is not followed and is no answer, even toward the market.
    let mut s = mockito::Server::new_async().await;
    let target = s.mock("POST", "/elsewhere").expect(0).create_async().await;
    let _m = s
        .mock("POST", "/api/upload-proof")
        .with_status(302)
        .with_header("location", &format!("{}/elsewhere", s.url()))
        .create_async()
        .await;
    let r = mainnet(&s).upload_proof(LISTING_FILE).await.unwrap();
    assert!(matches!(r, MarketReply::NoAnswer(_)), "{r:?}");
    target.assert_async().await;
}

/// R23: a file the client cannot name (not a mainnet listing file) is our
/// own refusal, before any HTTP.
#[tokio::test]
async fn upload_refuses_a_file_it_cannot_name() {
    let mut s = mockito::Server::new_async().await;
    let any = s
        .mock("POST", mockito::Matcher::Any)
        .expect(0)
        .create_async()
        .await;
    assert!(mainnet(&s).upload_proof("{}").await.is_err());
    any.assert_async().await;
}

/// R23 day 0: `POST /api/v2/pending-listings`, JSON with the name, network
/// `main`, the lock TRANSFER outpoint, the lock address and the mode; no
/// price (chosen at Finalize & sign) and no note.
#[tokio::test]
async fn pending_listing_shape() {
    let mut s = mockito::Server::new_async().await;
    let (m, seen) = recording(
        s.mock("POST", "/api/v2/pending-listings")
            .match_header("content-type", "application/json"),
        201,
        PENDING_ACCEPTED,
    );
    let _m = m.create_async().await;
    assert!(matches!(
        mainnet(&s).post_pending_listing(&pending()).await.unwrap(),
        MarketReply::Accepted(())
    ));
    let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({
            "name": "dexreviews",
            "network": "main",
            "transferTxHash": "abababababababababababababababababababababababababababababababab",
            "transferOutputIdx": 0,
            "lockScriptAddr": "hs1qx3lyaqwuhg7h6ut5rc9m89gna3jqce4ka07nvjmzljkq0696t2gqh449r6",
            "listingMode": "fixed-price"
        })
    );
    let reverse = PendingListing {
        kind: ListingKind::ReverseAuction,
        ..pending()
    };
    let _ = mainnet(&s).post_pending_listing(&reverse).await.unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[1]).unwrap();
    assert_eq!(sent["listingMode"], "reverse-auction");

    let mut s = mockito::Server::new_async().await;
    let _m = s
        .mock("POST", "/api/v2/pending-listings")
        .with_status(409)
        .with_body(r#"{"error":"A pending listing with that transferTxHash already exists"}"#)
        .create_async()
        .await;
    assert!(matches!(
        mainnet(&s).post_pending_listing(&pending()).await.unwrap(),
        MarketReply::Refused { status: 409, .. }
    ));
}

/// R28: `POST /api/v2/listings/<name>/refresh-status` with
/// `{outcome: "cancelled", cancelTxHash}` or `{saleTxHash}`; the market's
/// record reads as recorded, its "Listing not found" (400 or 404) as nothing
/// listed, its 404 "Transaction was not found" as not seen yet, its other
/// JSON refusals as refused, anything else as no answer.
#[tokio::test]
async fn refresh_status_shape() {
    let cancel_txid = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
    let sale_txid = "b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1";
    let mut s = mockito::Server::new_async().await;
    let (m, seen) = recording(
        s.mock("POST", "/api/v2/listings/dexreviews/refresh-status")
            .match_body(mockito::Matcher::PartialJson(
                serde_json::json!({"outcome": "cancelled"}),
            )),
        200,
        CANCEL_RECORDED,
    );
    let _m = m.create_async().await;
    let c = mainnet(&s);
    assert!(matches!(
        c.refresh_status("dexreviews", &StatusReport::Cancelled { cancel_txid })
            .await
            .unwrap(),
        MarketReply::Accepted(StatusRecorded::Recorded)
    ));
    let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({"outcome": "cancelled", "cancelTxHash": cancel_txid})
    );

    let mut s = mockito::Server::new_async().await;
    let (m, seen) = recording(
        s.mock("POST", "/api/v2/listings/dexreviews/refresh-status"),
        200,
        SALE_RECORDED,
    );
    let _m = m.create_async().await;
    assert!(matches!(
        mainnet(&s)
            .refresh_status("dexreviews", &StatusReport::Sold { sale_txid })
            .await
            .unwrap(),
        MarketReply::Accepted(StatusRecorded::Recorded)
    ));
    let sent: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0]).unwrap();
    assert_eq!(sent, serde_json::json!({"saleTxHash": sale_txid}));

    for (status, body, want) in [
        (400, NO_LISTING, "nothing"),
        (404, NO_LISTING, "nothing"),
        (404, TX_NOT_SEEN, "not seen"),
        (503, TX_NOT_SEEN, "none"),
        (400, r#"{"error":"Transaction was not found"}"#, "refused"),
        (404, "<!doctype html><h1>Not found</h1>", "none"),
        (
            400,
            r#"{"error":"Transaction does not spend this listing's Shakedex locking coin","url":"/listing/dexreviews"}"#,
            "refused",
        ),
        (503, r#"{"error":"Could not reach HSD node"}"#, "none"),
        (200, r#"{"sold":false,"status":"active"}"#, "none"),
    ] {
        let mut s = mockito::Server::new_async().await;
        let _m = s
            .mock("POST", "/api/v2/listings/dexreviews/refresh-status")
            .with_status(status)
            .with_body(body)
            .create_async()
            .await;
        let r = mainnet(&s)
            .refresh_status("dexreviews", &StatusReport::Cancelled { cancel_txid })
            .await
            .unwrap();
        match (want, &r) {
            ("nothing", MarketReply::Accepted(StatusRecorded::NoListing)) => {}
            ("refused", MarketReply::Refused { .. }) => {}
            ("not seen", MarketReply::NotSeenYet { status: 404, error }) => {
                assert_eq!(error, "Transaction was not found")
            }
            ("none", MarketReply::NoAnswer(_)) => {}
            _ => panic!("{status} {body}: {r:?}"),
        }
    }
}

/// R25: the market's copy, its own "not listed" (a JSON 404) and no answer
/// (an HTML 404, a 5xx, a body that is not JSON) are three answers.
#[tokio::test]
async fn proof_copy_tells_not_listed_from_no_answer() {
    for (status, body, want) in [
        (200, LISTING_FILE, "copy"),
        (404, PROOF_NOT_FOUND, "not listed"),
        (404, "<!doctype html><h1>Not found</h1>", "none"),
        (502, "<html>bad gateway</html>", "none"),
        (200, "<html>maintenance</html>", "none"),
    ] {
        let mut s = mockito::Server::new_async().await;
        let _m = s
            .mock("GET", "/listing/dexreviews/proof.json")
            .with_status(status)
            .with_body(body)
            .create_async()
            .await;
        let r = mainnet(&s).proof_copy("dexreviews").await.unwrap();
        match (want, &r) {
            ("copy", ProofCopy::Copy(text)) => assert_eq!(text, LISTING_FILE),
            ("not listed", ProofCopy::NotListed) => {}
            ("none", ProofCopy::NoAnswer(_)) => {}
            _ => panic!("{status} {body}: {r:?}"),
        }
    }
}

/// A market that cannot be reached (connection refused) gave no answer: each
/// write and the copy read say so, and none is our refusal or theirs.
#[tokio::test]
async fn unreachable_market_is_no_answer() {
    // Bind a port, then free it: nothing listens there.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let c = LearnHnsClient::with_base_url(&format!("http://127.0.0.1:{port}"))
        .unwrap()
        .for_network(Network::Main);
    let cancel = StatusReport::Cancelled {
        cancel_txid: "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
    };
    let upload = c.upload_proof(LISTING_FILE).await.unwrap();
    assert!(matches!(upload, MarketReply::NoAnswer(_)), "{upload:?}");
    let posted = c.post_pending_listing(&pending()).await.unwrap();
    assert!(matches!(posted, MarketReply::NoAnswer(_)), "{posted:?}");
    let reported = c.refresh_status("dexreviews", &cancel).await.unwrap();
    assert!(matches!(reported, MarketReply::NoAnswer(_)), "{reported:?}");
    let copy = c.proof_copy("dexreviews").await.unwrap();
    assert!(matches!(copy, ProofCopy::NoAnswer(_)), "{copy:?}");
}
