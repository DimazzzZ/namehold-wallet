use crate::market::learnhns::{
    market_fee_is_published, name_from_listing_link, FeeInfo, LearnHnsClient,
};

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
