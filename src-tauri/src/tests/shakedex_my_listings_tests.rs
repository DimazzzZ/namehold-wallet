//! "My listings" (T7): the rows the Market's My listings tab reads, what
//! each listing can do now and why not — the same refusal functions the
//! commands run — and its unsent draft.

use rusqlite::Connection;

use super::shakedex_sell_tests::{insert_draft, listing, store_conn};
use crate::commands::shakedex::{
    cancel_refusal, export_refusal, finalize_cancel_refusal, finalize_refusal, lower_refusal,
    my_listings_from_conn, ListingAction, MyListing,
};
use crate::db::queries::{self, ListingState, ShakedexListing};

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
    my_listings_from_conn(conn, "sell1")
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
    let ids: Vec<String> = my_listings_from_conn(&conn, "sell1")
        .unwrap()
        .into_iter()
        .map(|r| r.listing.id)
        .collect();
    assert_eq!(ids, ["new", "old"]);
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
        let l = if state == ListingState::Restored {
            listing("l", "dexsale", state) // no steps, no file
        } else {
            with_steps(listing("l", "dexsale", state))
        };
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
    }
}

/// One listing whose stored steps cannot be read does not hide the others:
/// its row is listed with every action refused, in the read's own words,
/// and nothing to send.
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
    assert_eq!(my_listings_from_conn(&conn, "sell1").unwrap().len(), 2);
    let row = row_of(&conn, "bad");
    let a = &row.actions;
    for action in [
        &a.finalize_and_sign,
        &a.lower_price,
        &a.cancel,
        &a.finalize_cancel,
        &a.export,
    ] {
        assert!(!allowed(action), "{action:?}");
        let why = action.reason.as_deref().unwrap();
        assert!(
            why.starts_with("corrupted listing bad: unreadable steps"),
            "{why}"
        );
    }
    assert!(row.listing.steps.is_empty());
    assert_eq!(row.pending_send, None);
    assert!(allowed(&row_of(&conn, "good").actions.lower_price));
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
    for (status, refused) in [("broadcasted", true), ("signed", true), ("failed", false)] {
        let conn = store_conn();
        insert_draft(&conn, "abort", "cancel", status);
        let mut l = listing("l", "dexsale", ListingState::ReadyToFinalize);
        l.abort_draft_id = Some("abort".into());
        insert(&conn, &l);
        let a = row_of(&conn, "l").actions.finalize_and_sign;
        assert_eq!(!a.allowed, refused, "{status}");
        if refused {
            assert_eq!(
                a.reason.as_deref(),
                Some(crate::noncustodial::shakedex::sell::CANCEL_TRANSFER_PENDING)
            );
        }
    }
}

/// R28: the cancel's FINALIZE waits for the lockup the last sync counted.
#[test]
fn finalize_cancel_waits_for_the_lockup_left_at_the_last_sync() {
    for (left, want) in [
        (Some(3), Some("the cancel can be finalized in 3 blocks")),
        (Some(1), Some("the cancel can be finalized in 1 block")),
        (Some(0), None),
        (None, None),
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

/// The listing's own draft that was never sent, for the row's Send button:
/// the lock TRANSFER while Locking, the FINALIZE into the lock while
/// Finalizing, the cancel while Cancelling, the cancel's FINALIZE while
/// CancelFinalizing. Every draft field holds its own unsent draft, so the
/// row names the one its state reads.
#[test]
fn pending_send_names_the_unsent_draft_of_the_state() {
    const DRAFTS: [(&str, &str); 4] = [
        ("dlock", "shakedex_lock"),
        ("dfin", "shakedex_lock_finalize"),
        ("dcan", "shakedex_cancel"),
        ("dcf", "shakedex_cancel_finalize"),
    ];
    // (state, the draft it reads, that draft's status, the row's `signed`)
    let cases = [
        (ListingState::Locking, "dlock", "signed", Some(true)),
        (ListingState::Locking, "dlock", "broadcasted", None),
        (ListingState::Finalizing, "dfin", "signed", Some(true)),
        (ListingState::Finalizing, "dfin", "broadcasted", None),
        (ListingState::Cancelling, "dcan", "signed", Some(true)),
        (ListingState::Cancelling, "dcan", "confirmed", None),
        (ListingState::CancelFinalizing, "dcf", "draft", Some(false)),
        (ListingState::CancelFinalizing, "dcf", "failed", None),
        (ListingState::Listed, "dfin", "signed", None),
    ];
    for (state, own, status, want) in cases {
        let conn = store_conn();
        for (id, action) in DRAFTS {
            let status = if id == own { status } else { "signed" };
            insert_draft(&conn, id, action, status);
            if status == "signed" {
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
            .map(|p| (p.draft_id, p.action, p.signed));
        let action = DRAFTS.iter().find(|(id, _)| *id == own).unwrap().1;
        let want = want.map(|signed| (own.to_string(), action.to_string(), signed));
        assert_eq!(got, want, "{state:?} {status}");
    }
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
