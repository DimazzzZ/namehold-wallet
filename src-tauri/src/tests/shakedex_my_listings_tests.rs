//! "My listings" (T7): the rows the Market's My listings tab reads, what
//! each listing can do now and why not — the same refusal functions the
//! commands run — and its unsent draft.

use rusqlite::Connection;

use super::shakedex_sell_tests::{insert_draft, listing, store_conn};
use crate::commands::shakedex::{
    cancel_refusal, export_refusal, finalize_cancel_refusal, finalize_refusal, import_refusal,
    lower_refusal, my_listings_from_conn, needs_median_time, publish_listing_from_conn,
    publish_refusal, save_listing_file_from_conn, ListingAction, ListingPrices, MyListing,
};
use crate::db::queries::{self, ListingState, ShakedexListing};
use crate::noncustodial::network::Network;

fn insert(conn: &Connection, l: &ShakedexListing) {
    queries::insert_shakedex_listing(conn, l).unwrap();
}

/// One stored step and a stored file, so the price and export rules see a
/// listing that has them.
fn with_steps(mut l: ShakedexListing) -> ShakedexListing {
    l.steps_json = r#"[{"price":5000000,"lockTime":1700000000,"signature":"00"}]"#.into();
    l.listing_file_json = Some("{}".into());
    l.lock_txid = Some("11".repeat(32));
    l.lock_vout = Some(0);
    l
}

fn row_of(conn: &Connection, id: &str) -> MyListing {
    my_listings_from_conn(conn, "sell1", Network::Main, None)
        .unwrap()
        .into_iter()
        .find(|r| r.listing.id == id)
        .expect("the listing's row")
}

fn allowed(a: &ListingAction) -> bool {
    assert_eq!(a.allowed, a.reason.is_none(), "{a:?}");
    a.allowed
}

#[test]
fn my_listings_lists_every_listing_of_the_profile_newest_first() {
    let conn = store_conn();
    insert(&conn, &listing("old", "alpha", ListingState::Sold));
    insert(&conn, &listing("new", "beta", ListingState::Locking));
    conn.execute(
        "UPDATE shakedex_listings SET created_at = '2026-01-01 00:00:00' WHERE id = 'old'",
        [],
    )
    .unwrap();
    queries::insert_wallet_profile(
        &conn,
        "other",
        "O",
        "mnemonic_hot",
        "regtest",
        "xpubO",
        0,
        false,
    )
    .unwrap();
    let mut theirs = listing("theirs", "gamma", ListingState::Listed);
    theirs.wallet_profile_id = "other".into();
    insert(&conn, &theirs);
    let ids: Vec<String> = my_listings_from_conn(&conn, "sell1", Network::Main, None)
        .unwrap()
        .into_iter()
        .map(|r| r.listing.id)
        .collect();
    assert_eq!(ids, ["new", "old"]);
}

/// R19: a Locking row carries the lockup left as the Owned Names row counts
/// it (`read_shakedex_listing_names`, one rule): from the lock TRANSFER
/// draft's confirmation height and the wallet's sync height; `None` before
/// the lock is seen mined, and in every other state.
#[test]
fn a_locking_row_counts_the_lockup_as_owned_names_does() {
    use crate::noncustodial::sync::set_sync_cursor;
    let params = Network::Regtest.name_params();
    let conn = store_conn();
    insert_draft(&conn, "dlock", "shakedex_lock", "confirmed");
    conn.execute(
        "UPDATE wallet_tx_drafts SET confirmation_height = 100 WHERE id = 'dlock'",
        [],
    )
    .unwrap();
    set_sync_cursor(&conn, "sell1", 101).unwrap();
    let mut l = listing("l", "dexsale", ListingState::Locking);
    l.lock_transfer_draft_id = Some("dlock".into());
    insert(&conn, &l);
    let left = row_of(&conn, "l").listing.blocks_until_finalize;
    assert_eq!(left, Some(params.blocks_until_finalize(100, 101)));
    assert_eq!(left, Some(8), "regtest lockup 10, judged at tip + 1");
    let owned = queries::read_shakedex_listing_names(&conn, "sell1", params, 101, 1).unwrap();
    assert_eq!(owned[0].listing.blocks_until_finalize, left, "one count");

    // Sent but not mined yet: no count.
    conn.execute(
        "UPDATE wallet_tx_drafts SET status = 'broadcasted' WHERE id = 'dlock'",
        [],
    )
    .unwrap();
    assert_eq!(row_of(&conn, "l").listing.blocks_until_finalize, None);
    // Past Locking: no count.
    conn.execute(
        "UPDATE wallet_tx_drafts SET status = 'confirmed' WHERE id = 'dlock'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE shakedex_listings SET state = 'ready_to_finalize'",
        [],
    )
    .unwrap();
    assert_eq!(row_of(&conn, "l").listing.blocks_until_finalize, None);
}

/// R35: a Listed or Restored row carries the blocks until buyers stop seeing
/// it, from the per-name count the node gave (some left, or 0 once past R9's
/// margin); `None` without a node's count and in every other state. Only
/// Listed and Restored names are asked of the node. The row also says the
/// network's warning window (R9's 30 days) and whether blocks keep time.
#[test]
fn a_listed_row_says_how_long_buyers_still_see_it() {
    use crate::commands::shakedex::{buyable_count_names, my_listings_from_conn_at};
    use std::collections::HashMap;
    let rows_with = |conn: &Connection, network: Network, left: &HashMap<String, i64>| {
        my_listings_from_conn_at(conn, "sell1", network, None, left).unwrap()
    };
    for state in ListingState::ALL {
        let conn = store_conn();
        insert(&conn, &listing("l", "dexsale", state));
        let counted = matches!(state, ListingState::Listed | ListingState::Restored);
        let names = buyable_count_names(&queries::list_shakedex_listings(&conn, "sell1").unwrap());
        assert_eq!(
            names,
            if counted {
                vec!["dexsale".to_string()]
            } else {
                vec![]
            },
            "{state:?}"
        );
        for left in [1_200, 0] {
            let by_name = HashMap::from([("dexsale".to_string(), left)]);
            let row = &rows_with(&conn, Network::Main, &by_name)[0];
            assert_eq!(
                row.listing.blocks_until_unbuyable,
                counted.then_some(left),
                "{state:?}"
            );
        }
        let unknown = &rows_with(&conn, Network::Main, &HashMap::new())[0];
        assert_eq!(unknown.listing.blocks_until_unbuyable, None, "{state:?}");
        let no_node = &my_listings_from_conn(&conn, "sell1", Network::Main, None).unwrap()[0];
        assert_eq!(no_node.listing.blocks_until_unbuyable, None, "{state:?}");
    }
    let conn = store_conn();
    insert(&conn, &listing("l", "dexsale", ListingState::Listed));
    let main = &rows_with(&conn, Network::Main, &HashMap::new())[0];
    assert!(main.wall_clock_timing);
    assert_eq!(main.unbuyable_warn_blocks, 4_320);
    let regtest = &rows_with(&conn, Network::Regtest, &HashMap::new())[0];
    assert!(!regtest.wall_clock_timing);
    assert_eq!(regtest.unbuyable_warn_blocks, 300);
    let testnet = &rows_with(&conn, Network::Testnet, &HashMap::new())[0];
    assert!(testnet.wall_clock_timing);
    assert_eq!(testnet.unbuyable_warn_blocks, 2_160, "half testnet's renewal window");
}

/// Which state allows which action: Finalize & sign only ReadyToFinalize,
/// Lower price only Listed, Cancel Listed and Restored, Finalize cancel
/// only CancelAwaitingFinalize, export the EXPORT_ALLOWED states with a file.
/// Every action's reason is the output of the refusal function its command
/// runs first.
#[test]
fn listing_actions_follow_the_commands_refusals_in_every_state() {
    for state in ListingState::ALL {
        let conn = store_conn();
        let mut l = if state == ListingState::Restored {
            listing("l", "dexsale", state) // no steps, no file
        } else {
            with_steps(listing("l", "dexsale", state))
        };
        // A mined cancel's lockup counted and over at the last sync.
        if ListingState::CANCEL_MINED.contains(&state) {
            l.cancel_blocks_remaining = Some(0);
        }
        insert(&conn, &l);
        let a = row_of(&conn, "l").actions;
        assert_eq!(
            allowed(&a.finalize_and_sign),
            state == ListingState::ReadyToFinalize,
            "{state:?}"
        );
        assert_eq!(
            allowed(&a.lower_price),
            state == ListingState::Listed,
            "{state:?}"
        );
        assert_eq!(
            allowed(&a.cancel),
            matches!(state, ListingState::Listed | ListingState::Restored),
            "{state:?}"
        );
        assert_eq!(
            allowed(&a.finalize_cancel),
            state == ListingState::CancelAwaitingFinalize,
            "{state:?}"
        );
        assert_eq!(
            allowed(&a.export),
            ListingState::EXPORT_ALLOWED.contains(&state),
            "{state:?}"
        );
        let words = |o: Option<&str>| o.map(str::to_string);
        assert_eq!(
            a.finalize_and_sign.reason,
            words(finalize_refusal(state)),
            "{state:?}"
        );
        let no_steps = state == ListingState::Restored;
        assert_eq!(
            a.lower_price.reason,
            words(lower_refusal(state, no_steps)),
            "{state:?}"
        );
        assert_eq!(a.cancel.reason, words(cancel_refusal(state)), "{state:?}");
        assert_eq!(
            a.finalize_cancel.reason,
            words(finalize_cancel_refusal(state)),
            "{state:?}"
        );
        assert_eq!(a.export.reason, words(export_refusal(&l)), "{state:?}");
        assert_eq!(
            allowed(&a.import_file),
            state == ListingState::Restored,
            "{state:?}"
        );
        assert_eq!(
            a.import_file.reason,
            words(import_refusal(state)),
            "{state:?}"
        );
        // `with_steps` stores an unreadable file ("{}"): refused in every
        // state, each in the words of `publish_refusal`.
        assert!(!allowed(&a.publish), "{state:?}");
        assert_eq!(
            a.publish.reason,
            publish_refusal(&l, Network::Main),
            "{state:?}"
        );
    }
}

/// One listing whose stored steps cannot be read does not hide the others:
/// its row is listed with every action that reads them refused, in the
/// read's own words, and nothing to send. Importing its listing file reads
/// no stored step (the file replaces them): it keeps its own refusal.
#[test]
fn an_unreadable_listing_is_listed_with_every_action_refused() {
    let conn = store_conn();
    insert(
        &conn,
        &with_steps(listing("good", "alpha", ListingState::Listed)),
    );
    let mut bad = with_steps(listing("bad", "beta", ListingState::Cancelling));
    bad.steps_json = "not json".into();
    insert_draft(&conn, "dcan", "shakedex_cancel", "signed");
    bad.cancel_draft_id = Some("dcan".into());
    insert(&conn, &bad);
    assert_eq!(
        my_listings_from_conn(&conn, "sell1", Network::Main, None)
            .unwrap()
            .len(),
        2
    );
    let row = row_of(&conn, "bad");
    let a = &row.actions;
    for action in [
        &a.finalize_and_sign,
        &a.lower_price,
        &a.cancel,
        &a.finalize_cancel,
        &a.export,
        &a.publish,
    ] {
        assert!(!allowed(action), "{action:?}");
        let why = action.reason.as_deref().unwrap();
        assert!(
            why.starts_with("corrupted listing bad: unreadable steps"),
            "{why}"
        );
    }
    assert_eq!(
        a.import_file.reason.as_deref(),
        import_refusal(ListingState::Cancelling)
    );
    assert!(row.listing.steps.is_empty());
    assert!(
        matches!(&row.prices, ListingPrices::Unreadable { reason } if reason.starts_with("corrupted listing bad")),
        "{:?}",
        row.prices
    );
    assert_eq!(row.pending_send, None);
    assert!(allowed(&row_of(&conn, "good").actions.lower_price));
}

/// A Restored lock whose stored steps cannot be read can still take its
/// listing file: the import refuses only what is not a Restored lock
/// (`import_refusal`), and every other action of the row is refused in the
/// read's words. The import itself is pinned by
/// `own_listing_file_upgrades_a_restored_lock_whose_stored_steps_are_unreadable`.
#[test]
fn an_unreadable_restored_lock_can_still_import_its_listing_file() {
    let conn = store_conn();
    let mut bad = listing("bad", "beta", ListingState::Restored);
    bad.steps_json = "not json".into();
    insert(&conn, &bad);
    let row = row_of(&conn, "bad");
    assert!(allowed(&row.actions.import_file));
    assert!(!allowed(&row.actions.cancel));
    assert!(matches!(row.prices, ListingPrices::Unreadable { .. }));
}

#[test]
fn lower_price_is_refused_while_cancelling_in_the_commands_words() {
    let conn = store_conn();
    insert(
        &conn,
        &with_steps(listing("l", "dexsale", ListingState::Cancelling)),
    );
    let a = row_of(&conn, "l").actions.lower_price;
    assert_eq!(
        a.reason.as_deref(),
        Some(crate::commands::shakedex::LOWER_DURING_CANCEL)
    );
}

#[test]
fn a_listed_listing_without_signed_prices_cannot_be_lowered() {
    let conn = store_conn();
    insert(&conn, &listing("l", "dexsale", ListingState::Listed));
    let a = row_of(&conn, "l").actions.lower_price;
    assert_eq!(
        a.reason.as_deref(),
        Some(crate::commands::shakedex::LOWER_NO_PRICE_HERE)
    );
}

/// R23: a Listed lock whose file is not here (finalized by another wallet,
/// its file not imported) has nothing to export, in the command's words.
#[test]
fn export_is_refused_for_a_listed_lock_without_its_file() {
    let conn = store_conn();
    let mut l = with_steps(listing("l", "dexsale", ListingState::Listed));
    l.listing_file_json = None;
    insert(&conn, &l);
    let a = row_of(&conn, "l").actions.export;
    let command = crate::commands::shakedex::export_listing_file_from_conn(&conn, "sell1", "l")
        .unwrap_err()
        .to_string();
    let reason = a.reason.expect("export refused");
    assert!(command.contains(&reason), "{command} / {reason}");
}

/// R19: the Cancel transfer and the FINALIZE spend one coin; Finalize &
/// sign waits while the cancel may still land, and not after it failed.
#[test]
fn finalize_and_sign_is_refused_while_its_cancel_transfer_may_land() {
    use crate::noncustodial::shakedex::sell::CANCEL_TRANSFER_PENDING;
    // During the lockup too: the pending abort is the reason that matters,
    // since once it lands the listing never opens Finalize & sign at all.
    for state in [ListingState::Locking, ListingState::ReadyToFinalize] {
        for (status, pending) in [("broadcasted", true), ("signed", true), ("failed", false)] {
            let conn = store_conn();
            insert_draft(&conn, "abort", "cancel", status);
            let mut l = listing("l", "dexsale", state);
            l.abort_draft_id = Some("abort".into());
            insert(&conn, &l);
            let a = row_of(&conn, "l").actions.finalize_and_sign;
            let want = if pending {
                Some(CANCEL_TRANSFER_PENDING)
            } else {
                finalize_refusal(state)
            };
            assert_eq!(a.reason.as_deref(), want, "{state:?} {status}");
            assert_eq!(a.allowed, want.is_none(), "{state:?} {status}");
        }
    }
}

/// R28: the cancel's FINALIZE waits for the lockup the last sync counted;
/// with no count taken yet it is refused, never offered on a guess.
#[test]
fn finalize_cancel_waits_for_the_lockup_left_at_the_last_sync() {
    for (left, want) in [
        (Some(3), Some("the cancel can be finalized in 3 blocks")),
        (Some(1), Some("the cancel can be finalized in 1 block")),
        (Some(0), None),
        (
            None,
            Some(crate::commands::shakedex::CANCEL_LOCKUP_NOT_COUNTED),
        ),
    ] {
        let conn = store_conn();
        let mut l = with_steps(listing(
            "l",
            "dexsale",
            ListingState::CancelAwaitingFinalize,
        ));
        l.cancel_blocks_remaining = left;
        insert(&conn, &l);
        let a = row_of(&conn, "l").actions.finalize_cancel;
        assert_eq!(a.reason.as_deref(), want, "{left:?}");
    }
}

/// The listing's own draft that was never sent, or whose send the node may
/// not hold (`broadcast_pending`), for the row's Send button: the lock
/// TRANSFER while Locking, the FINALIZE into the lock while Finalizing, the
/// cancel while Cancelling, the cancel's FINALIZE while CancelFinalizing.
/// Every draft field holds its own unsent draft, so the row names the one
/// its state reads. Only a draft never sent can be discarded
/// (`delete_tx_draft` refuses a `broadcast_pending` one).
#[test]
fn pending_send_names_the_unsent_draft_of_the_state() {
    const DRAFTS: [(&str, &str); 4] = [
        ("dlock", "shakedex_lock"),
        ("dfin", "shakedex_lock_finalize"),
        ("dcan", "shakedex_cancel"),
        ("dcf", "shakedex_cancel_finalize"),
    ];
    // (state, the draft it reads, that draft's status, the row's
    // `(signed, discardable)`)
    let cases = [
        (ListingState::Locking, "dlock", "signed", Some((true, true))),
        (
            ListingState::Locking,
            "dlock",
            "broadcast_pending",
            Some((true, false)),
        ),
        (ListingState::Locking, "dlock", "broadcasted", None),
        (
            ListingState::Finalizing,
            "dfin",
            "signed",
            Some((true, true)),
        ),
        (
            ListingState::Finalizing,
            "dfin",
            "broadcast_pending",
            Some((true, false)),
        ),
        (ListingState::Finalizing, "dfin", "broadcasted", None),
        (
            ListingState::Cancelling,
            "dcan",
            "signed",
            Some((true, true)),
        ),
        (
            ListingState::Cancelling,
            "dcan",
            "broadcast_pending",
            Some((true, false)),
        ),
        (ListingState::Cancelling, "dcan", "confirmed", None),
        (
            ListingState::CancelFinalizing,
            "dcf",
            "draft",
            Some((false, true)),
        ),
        (
            ListingState::CancelFinalizing,
            "dcf",
            "broadcast_pending",
            Some((true, false)),
        ),
        (ListingState::CancelFinalizing, "dcf", "failed", None),
        (ListingState::Listed, "dfin", "signed", None),
    ];
    for (state, own, status, want) in cases {
        let conn = store_conn();
        for (id, action) in DRAFTS {
            let status = if id == own { status } else { "signed" };
            insert_draft(&conn, id, action, status);
            if status == "signed" || status == "broadcast_pending" {
                conn.execute(
                    "UPDATE wallet_tx_drafts SET signed_tx_hex = '00' WHERE id = ?1",
                    [id],
                )
                .unwrap();
            }
        }
        let mut l = with_steps(listing("l", "dexsale", state));
        l.lock_transfer_draft_id = Some("dlock".into());
        l.lock_finalize_draft_id = Some("dfin".into());
        l.cancel_draft_id = Some("dcan".into());
        l.cancel_finalize_draft_id = Some("dcf".into());
        insert(&conn, &l);
        let got = row_of(&conn, "l")
            .pending_send
            .map(|p| (p.draft_id, p.action, p.signed, p.discardable));
        let action = DRAFTS.iter().find(|(id, _)| *id == own).unwrap().1;
        let want = want.map(|(signed, discardable)| {
            (own.to_string(), action.to_string(), signed, discardable)
        });
        assert_eq!(got, want, "{state:?} {status}");
    }
}

/// Why a row has no price is a variant of its own: no steps here, no node
/// that can say the median time (SPV, Explorer), or a node that failed.
#[test]
fn listing_prices_say_why_they_are_missing() {
    let conn = store_conn();
    insert(
        &conn,
        &with_steps(listing("s", "dexsale", ListingState::Listed)),
    );
    insert(&conn, &listing("n", "other", ListingState::Restored));
    let prices = |chain: Option<Result<u64, String>>, id: &str| {
        my_listings_from_conn(&conn, "sell1", Network::Main, chain)
            .unwrap()
            .into_iter()
            .find(|r| r.listing.id == id)
            .unwrap()
            .prices
    };
    assert_eq!(prices(Some(Ok(1_700_000_000)), "n"), ListingPrices::NoSteps);
    assert_eq!(prices(None, "n"), ListingPrices::NoSteps);
    assert_eq!(prices(None, "s"), ListingPrices::Unverified);
    assert_eq!(
        prices(Some(Err("node down".into())), "s"),
        ListingPrices::CouldNotCheck {
            reason: "node down".into()
        }
    );
    assert_eq!(
        prices(Some(Ok(1_700_000_000)), "s"),
        ListingPrices::At {
            current_price: Some(5_000_000),
            next_price: None,
            next_valid_in_secs: None,
            floor_price: 5_000_000,
        }
    );
}

/// A stored lock time past R3's 40 bits is a corrupted row, whether or not
/// a node can say the median time: every action is refused and its prices
/// are unreadable, in the read's words, and the other rows keep theirs.
#[test]
fn a_listing_with_an_unreadable_lock_time_is_listed_with_nothing_allowed() {
    let conn = store_conn();
    insert(
        &conn,
        &with_steps(listing("good", "alpha", ListingState::Listed)),
    );
    let mut bad = with_steps(listing("bad", "beta", ListingState::Listed));
    bad.steps_json = r#"[{"price":1,"lockTime":2199023255552,"signature":"00"}]"#.into();
    insert(&conn, &bad);
    for chain in [None, Some(Err("node down".into())), Some(Ok(1_700_000_000))] {
        let rows = my_listings_from_conn(&conn, "sell1", Network::Main, chain.clone()).unwrap();
        let of = |id: &str| rows.iter().find(|r| r.listing.id == id).unwrap();
        let row = of("bad");
        let ListingPrices::Unreadable { reason } = &row.prices else {
            panic!("{chain:?}: {:?}", row.prices)
        };
        assert!(reason.starts_with("corrupted listing"), "{reason}");
        let a = &row.actions;
        for action in [
            &a.finalize_and_sign,
            &a.lower_price,
            &a.cancel,
            &a.finalize_cancel,
            &a.export,
            &a.publish,
        ] {
            assert_eq!(action.reason.as_ref(), Some(reason), "{chain:?}");
        }
        assert!(allowed(&of("good").actions.lower_price), "{chain:?}");
    }
}

const LISTING_FILE: &str = include_str!("../../tests/vectors/shakedex/proof_dexreviews.json");

/// A Listed listing with the vector's mainnet file, unpublished.
fn listed_with_file(file: &str) -> ShakedexListing {
    let mut l = with_steps(listing("l", "dexreviews", ListingState::Listed));
    l.listing_file_json = Some(file.into());
    l
}

fn file_with_fee(fee: u64) -> String {
    let mut j: serde_json::Value = serde_json::from_str(LISTING_FILE).unwrap();
    j["data"][0]["fee"] = fee.into();
    j.to_string()
}

/// Carry 1: publishing is turned on later only for a Listed listing that
/// is not published yet, on mainnet, with its own readable file here.
#[test]
fn publish_refusal_allows_only_an_unpublished_listed_listing_on_mainnet() {
    let base = listed_with_file(&file_with_fee(0));
    assert_eq!(publish_refusal(&base, Network::Main), None);
    for network in [Network::Regtest, Network::Testnet] {
        assert_eq!(
            publish_refusal(&base, network).as_deref(),
            Some(crate::commands::shakedex::MARKET_MAINNET_ONLY),
            "{network:?}"
        );
    }
    let mut published = base.clone();
    published.publish = true;
    let why = publish_refusal(&published, Network::Main).expect("refused");
    assert!(why.contains("already published"), "{why}");
    for state in ListingState::ALL
        .into_iter()
        .filter(|s| *s != ListingState::Listed)
    {
        let mut l = base.clone();
        l.state = state;
        assert!(publish_refusal(&l, Network::Main).is_some(), "{state:?}");
    }
    let mut no_file = base.clone();
    no_file.listing_file_json = None;
    let why = publish_refusal(&no_file, Network::Main).expect("refused");
    assert!(why.contains("import its saved file"), "{why}");
    let mut unreadable = base.clone();
    unreadable.listing_file_json = Some("{}".into());
    let why = publish_refusal(&unreadable, Network::Main).expect("refused");
    assert!(why.contains("cannot be read"), "{why}");
}

/// Carry 2 (T6 s2): a file with a market fee in effect never matches the
/// market's copy (R25), so it would be uploaded again every hour.
#[test]
fn publishing_a_listing_whose_file_names_a_market_fee_is_refused() {
    let l = listed_with_file(&file_with_fee(1_000));
    let why = publish_refusal(&l, Network::Main).expect("refused");
    assert!(why.contains("market fee"), "{why}");
    let mut j: serde_json::Value = serde_json::from_str(&file_with_fee(0)).unwrap();
    j["feeAddr"] = "hs1qfee".into();
    let l = listed_with_file(&j.to_string());
    let why = publish_refusal(&l, Network::Main).expect("refused");
    assert!(why.contains("market fee"), "{why}");
}

#[test]
fn publish_listing_turns_publish_on_and_the_market_jobs_keep_it() {
    let conn = store_conn();
    insert(&conn, &listed_with_file(&file_with_fee(0)));
    let before = my_listings_from_conn(&conn, "sell1", Network::Main, None)
        .unwrap()
        .remove(0);
    assert!(allowed(&before.actions.publish));
    assert!(queries::list_listings_kept_on_market(&conn, "sell1")
        .unwrap()
        .is_empty());
    let s = publish_listing_from_conn(&conn, "sell1", Network::Main, "l").unwrap();
    assert!(s.publish);
    assert_eq!(
        queries::list_listings_kept_on_market(&conn, "sell1")
            .unwrap()
            .len(),
        1
    );
    // Once published, My listings offers no publish, in the refusal's words.
    let row = my_listings_from_conn(&conn, "sell1", Network::Main, None)
        .unwrap()
        .remove(0);
    assert!(!allowed(&row.actions.publish));
    assert!(row
        .actions
        .publish
        .reason
        .as_deref()
        .unwrap()
        .contains("already published"));
    let e = publish_listing_from_conn(&conn, "sell1", Network::Main, "l").unwrap_err();
    assert!(e.to_string().contains("already published"), "{e}");
}

/// A refused publish writes nothing; another profile's listing is not found.
#[test]
fn a_refused_publish_writes_nothing() {
    let conn = store_conn();
    insert(&conn, &listed_with_file(&file_with_fee(1_000)));
    assert!(publish_listing_from_conn(&conn, "sell1", Network::Main, "l").is_err());
    assert!(publish_listing_from_conn(&conn, "sell1", Network::Regtest, "l").is_err());
    assert!(publish_listing_from_conn(&conn, "other", Network::Main, "l").is_err());
    let l = queries::get_shakedex_listing(&conn, "l").unwrap().unwrap();
    assert!(!l.publish);
}

/// The database write guards the same facts again: a listing that changed
/// state after the refusal was asked is not published.
#[test]
fn publish_write_takes_only_an_unpublished_listed_row() {
    let conn = store_conn();
    insert(&conn, &listed_with_file(&file_with_fee(0)));
    conn.execute(
        "UPDATE shakedex_listings SET state = 'cancelling' WHERE id = 'l'",
        [],
    )
    .unwrap();
    assert_eq!(
        queries::publish_shakedex_listing(&conn, "sell1", "l").unwrap(),
        0
    );
    assert_eq!(
        queries::publish_shakedex_listing(&conn, "other", "l").unwrap(),
        0
    );
    conn.execute(
        "UPDATE shakedex_listings SET state = 'listed', publish = 1 WHERE id = 'l'",
        [],
    )
    .unwrap();
    assert_eq!(
        queries::publish_shakedex_listing(&conn, "sell1", "l").unwrap(),
        0
    );
}

#[test]
fn save_listing_file_writes_the_stored_file_as_written_and_nothing_when_refused() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = |tag: &str| {
        std::env::temp_dir().join(format!(
            "namehold_listing_{tag}_{}_{}.json",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ))
    };
    let conn = store_conn();
    let file = file_with_fee(0);
    insert(&conn, &listed_with_file(&file));
    let ok = path("ok");
    save_listing_file_from_conn(&conn, "sell1", "l", &ok).unwrap();
    assert_eq!(std::fs::read_to_string(&ok).unwrap(), file);
    std::fs::remove_file(&ok).unwrap();

    let other = path("other");
    assert!(save_listing_file_from_conn(&conn, "other", "l", &other).is_err());
    assert!(!other.exists());

    conn.execute(
        "UPDATE shakedex_listings SET state = 'finalizing' WHERE id = 'l'",
        [],
    )
    .unwrap();
    let refused = path("refused");
    assert!(save_listing_file_from_conn(&conn, "sell1", "l", &refused).is_err());
    assert!(!refused.exists());
}

/// F3: the export path comes from the webview (SECURITY.md §1): a symbolic
/// link there is refused and the file it points to is left as it was.
#[cfg(unix)]
#[test]
fn save_listing_file_to_a_symlink_is_refused_and_its_target_untouched() {
    let base = std::env::temp_dir().join(format!("namehold_listing_link_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir(&base).unwrap();
    let target = base.join("target.txt");
    std::fs::write(&target, "the user's own file").unwrap();
    let link = base.join("listing.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let conn = store_conn();
    insert(&conn, &listed_with_file(&file_with_fee(0)));

    let err = save_listing_file_from_conn(&conn, "sell1", "l", &link).unwrap_err();
    let content = std::fs::read_to_string(&target).unwrap();
    std::fs::remove_dir_all(&base).unwrap();
    assert!(
        err.to_string()
            .contains(crate::commands::user_file::SAVE_TO_SYMLINK),
        "{err:?}"
    );
    assert_eq!(content, "the user's own file");
}

/// The TS mirrors (`src/types/index.ts`) list exactly the Rust fields.
#[test]
fn ts_my_listing_mirrors_list_every_field() {
    let conn = store_conn();
    insert(&conn, &listing("l", "dexsale", ListingState::Locking));
    let row = serde_json::to_value(row_of(&conn, "l")).unwrap();
    let send = serde_json::to_value(crate::commands::shakedex::PendingSend {
        draft_id: "d1".into(),
        action: "shakedex_cancel".into(),
        signed: true,
        discardable: true,
    })
    .unwrap();
    for (iface, json) in [
        ("ShakedexMyListing", &row),
        ("ShakedexListingActions", &row["actions"]),
        ("ShakedexListingAction", &row["actions"]["cancel"]),
        ("ShakedexPendingSend", &send),
    ] {
        let mut rust: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        rust.sort_unstable();
        assert_eq!(ts_interface_keys(iface), rust, "{iface}");
    }
}

/// `ListingPrices` serializes to exactly the members of the TS union
/// `ShakedexListingPrices`: each variant's `kind` tag and its other keys,
/// and every TS member has a Rust variant.
#[test]
fn ts_listing_prices_mirror_every_variant() {
    let variants = [
        ListingPrices::NoSteps,
        ListingPrices::Unverified,
        ListingPrices::CouldNotCheck { reason: "r".into() },
        ListingPrices::Unreadable { reason: "r".into() },
        ListingPrices::At {
            current_price: Some(1),
            next_price: None,
            next_valid_in_secs: None,
            floor_price: 1,
        },
    ];
    let mut rust: Vec<(String, Vec<String>)> = variants
        .iter()
        .map(|v| {
            let json = serde_json::to_value(v).unwrap();
            let obj = json.as_object().unwrap();
            let kind = obj["kind"].as_str().unwrap().to_string();
            let mut keys: Vec<String> = obj.keys().filter(|k| *k != "kind").cloned().collect();
            keys.sort_unstable();
            (kind, keys)
        })
        .collect();
    rust.sort_unstable();
    let mut ts = ts_union_members("ShakedexListingPrices");
    ts.sort_unstable();
    assert_eq!(rust, ts);
    let kinds: Vec<&str> = rust.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        kinds,
        ["at", "couldNotCheck", "noSteps", "unreadable", "unverified"]
    );
    let reason = vec!["reason".to_string()];
    assert_eq!(rust[1].1, reason, "couldNotCheck");
    assert_eq!(rust[3].1, reason, "unreadable");
}

/// `(kind, other keys sorted)` of each `| { kind: "…"; … }` member of
/// `export type <name> =` in `src/types/index.ts`.
pub(super) fn ts_union_members(name: &str) -> Vec<(String, Vec<String>)> {
    const TS: &str = include_str!("../../../src/types/index.ts");
    let start = TS
        .find(&format!("export type {name} ="))
        .unwrap_or_else(|| panic!("type {name}"));
    let block = &TS[start..start + TS[start..].find("\n\n").unwrap()];
    block
        .split("| {")
        .skip(1)
        .map(|member| {
            let body = &member[..member.find('}').expect("member end")];
            let mut kind = None;
            let mut keys = Vec::new();
            for field in body.split([';', '\n']).map(str::trim) {
                let Some((k, v)) = field.split_once(':') else {
                    continue;
                };
                let k = k.trim();
                if k == "kind" {
                    kind = Some(v.trim().trim_matches('"').to_string());
                } else {
                    keys.push(k.to_string());
                }
            }
            keys.sort_unstable();
            (kind.expect("kind"), keys)
        })
        .collect()
}

/// The list reads the node's median time only when a readable row has
/// steps: no rows, rows without steps, or only an unreadable row need none.
#[test]
fn the_median_time_is_needed_only_for_a_readable_row_with_steps() {
    let none = listing("n", "alpha", ListingState::Restored);
    let mut bad = with_steps(listing("b", "beta", ListingState::Listed));
    bad.steps_json = "not json".into();
    let good = with_steps(listing("g", "gamma", ListingState::Listed));
    assert!(!needs_median_time(&[]));
    assert!(!needs_median_time(std::slice::from_ref(&none)));
    assert!(!needs_median_time(&[none.clone(), bad.clone()]));
    assert!(needs_median_time(&[none, bad, good]));
}

/// Publishing a corrupted row is refused as its row's publish action is,
/// in the same words, and writes nothing: unreadable steps, or a stored
/// lock time past 40 bits.
#[test]
fn publishing_a_corrupted_row_is_refused_in_the_rows_words() {
    for steps in [
        "not json",
        r#"[{"price":1,"lockTime":2199023255552,"signature":"00"}]"#,
    ] {
        let conn = store_conn();
        let mut l = listed_with_file(&file_with_fee(0));
        l.steps_json = steps.into();
        insert(&conn, &l);
        let row = my_listings_from_conn(&conn, "sell1", Network::Main, None)
            .unwrap()
            .remove(0);
        let e = publish_listing_from_conn(&conn, "sell1", Network::Main, "l").unwrap_err();
        assert_eq!(Some(e.to_string()), row.actions.publish.reason, "{steps}");
        assert!(
            !queries::get_shakedex_listing(&conn, "l")
                .unwrap()
                .unwrap()
                .publish,
            "{steps}"
        );
    }
}

/// The field names of `export interface <name> {` in `src/types/index.ts`.
pub(super) fn ts_interface_keys(name: &str) -> Vec<&'static str> {
    const TS: &str = include_str!("../../../src/types/index.ts");
    let start = TS
        .find(&format!("export interface {name} {{"))
        .unwrap_or_else(|| panic!("interface {name}"));
    let block = &TS[start..start + TS[start..].find("\n}").unwrap()];
    let mut keys: Vec<&str> = block
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.starts_with("/**") && !l.starts_with('*') && !l.is_empty())
        .filter_map(|l| l.split_once(':').map(|(k, _)| k.trim()))
        .collect();
    keys.sort_unstable();
    keys
}

/// Where the browser QA mock reads what each listing can do
/// (`src/lib/webqa-mock.ts`), so it never keeps copies of the refusal
/// sentences that can drift from the commands'.
const ACTIONS_FIXTURE: &str = "../src/test/fixtures/listing-actions.json";

/// A number that appears nowhere else in the wait sentence, replaced by
/// `{n}` so the mock fills in its own count.
const WAIT_PLACEHOLDER: i64 = 987_654;

/// The fixture as the backend computes it now: every state's actions on
/// mainnet, with and without stored steps and a listing file (a fee-free
/// one), published or not; the lockup sentence of a mined cancel; Finalize &
/// sign's refusal while a Cancel transfer may still land (R19); and the
/// owner actions' R27 refusal.
fn listing_actions_fixture() -> serde_json::Value {
    let mut cases = Vec::new();
    for state in ListingState::ALL {
        for with_file in [false, true] {
            for published in [false, true] {
                let conn = store_conn();
                let mut l = if with_file {
                    let mut l = with_steps(listing("l", "dexreviews", state));
                    l.listing_file_json = Some(file_with_fee(0));
                    l
                } else {
                    listing("l", "dexreviews", state)
                };
                l.publish = published;
                // A mined cancel's lockup counted and over: the mock fills
                // in the count it keeps (`cancelFinalizeWait`).
                if ListingState::CANCEL_MINED.contains(&state) {
                    l.cancel_blocks_remaining = Some(0);
                }
                insert(&conn, &l);
                cases.push(serde_json::json!({
                    "state": state,
                    "withFile": with_file,
                    "published": published,
                    "actions": row_of(&conn, "l").actions,
                }));
            }
        }
    }
    serde_json::json!({
        "_about": "What each listing can do now, as commands/shakedex.rs computes it on mainnet. Generated by shakedex_my_listings_tests::listing_actions_fixture_matches_the_backend; regenerate with UPDATE_FIXTURES=1 cargo test --manifest-path src-tauri/Cargo.toml --locked --lib listing_actions_fixture, then npx prettier --write src/test/fixtures/listing-actions.json.",
        "nameLockedForSale": crate::noncustodial::shakedex::NAME_LOCKED_FOR_SALE,
        "cancelTransferPending": crate::noncustodial::shakedex::sell::CANCEL_TRANSFER_PENDING,
        "cancelFinalizeWaitOne": crate::commands::shakedex::cancel_finalize_wait(1),
        "cancelFinalizeWait": crate::commands::shakedex::cancel_finalize_wait(WAIT_PLACEHOLDER)
            .replace(&WAIT_PLACEHOLDER.to_string(), "{n}"),
        "cancelLockupNotCounted": crate::commands::shakedex::CANCEL_LOCKUP_NOT_COUNTED,
        "cases": cases,
    })
}

/// The browser QA mock's fixture is the backend's own output: a refusal
/// sentence or an allowed set changed here without regenerating it fails.
#[test]
fn listing_actions_fixture_matches_the_backend() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(ACTIONS_FIXTURE);
    let live = listing_actions_fixture();
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        let text = serde_json::to_string_pretty(&live).unwrap() + "\n";
        std::fs::write(&path, text).unwrap();
    }
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the fixture exists"))
            .expect("the fixture is JSON");
    assert!(
        stored == live,
        "src/test/fixtures/listing-actions.json differs from the backend's listing actions; \
         regenerate with UPDATE_FIXTURES=1 cargo test --manifest-path src-tauri/Cargo.toml \
         --locked --lib listing_actions_fixture"
    );
}

/// R7 (per-wallet read isolation, QA Q2): My listings reads the profile the
/// caller names, not the backend's active one, so a read issued for one
/// profile never answers with another's rows across a switch; with none
/// named it reads the active profile.
#[tokio::test]
async fn list_listings_reads_the_profile_it_is_given() {
    use crate::tests::shakedex_cmd_tests::{app_with, seeded, PROFILE};
    let conn = seeded("regtest", "mnemonic_hot", "http://127.0.0.1:1");
    queries::insert_wallet_profile(
        &conn,
        "other",
        "Other",
        "mnemonic_hot",
        "regtest",
        "xpubOTHER",
        0,
        false,
    )
    .unwrap();
    let mut mine = listing("mine", "alpha", ListingState::Restored);
    mine.wallet_profile_id = PROFILE.into();
    insert(&conn, &mine);
    let mut theirs = listing("theirs", "beta", ListingState::Restored);
    theirs.wallet_profile_id = "other".into();
    insert(&conn, &theirs);
    let app = app_with(conn);
    let ids = |rows: Vec<MyListing>| rows.into_iter().map(|r| r.listing.id).collect::<Vec<_>>();
    let other = crate::commands::shakedex::shakedex_list_listings(
        tauri::Manager::state(&app),
        Some("other".into()),
    )
    .await
    .unwrap();
    assert_eq!(
        ids(other),
        ["theirs"],
        "the named profile, not the active one"
    );
    let active =
        crate::commands::shakedex::shakedex_list_listings(tauri::Manager::state(&app), None)
            .await
            .unwrap();
    assert_eq!(ids(active), ["mine"], "none named: the active profile");
}
