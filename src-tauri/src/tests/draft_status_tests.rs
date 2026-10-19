//! `wallet_tx_drafts.status` is a [`DraftStatus`]: stored by its spelling,
//! read back as the same variant, an unknown spelling is an error, and the
//! table's CHECK lists exactly the spellings the enum has.

use rusqlite::{params, Connection};

use crate::db::queries::{self, DraftStatus};

const PROFILE: &str = "p1";

/// The spellings, as migration 012's CHECK and the frontend's
/// `TxDraft.status` union (`src/types/index.ts`) write them.
const SPELLINGS: [&str; 7] = [
    "draft",
    "signed",
    "broadcast_pending",
    "broadcasted",
    "confirmed",
    "failed",
    "dropped",
];

fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();
    queries::insert_wallet_profile(
        &conn,
        PROFILE,
        "Primary",
        "mnemonic_hot",
        "regtest",
        "xpubX",
        0,
        false,
    )
    .unwrap();
    conn
}

fn insert_raw(conn: &Connection, id: &str, status: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO wallet_tx_drafts
            (id, wallet_profile_id, action, unsigned_tx_hex, signing_inputs_json,
             summary_json, status)
         VALUES (?1, ?2, 'send_hns', '00', '[]', '{}', ?3)",
        params![id, PROFILE, status],
    )
}

#[test]
fn every_status_has_its_spelling_in_the_db_and_on_the_wire() {
    let spelled: Vec<&str> = DraftStatus::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(spelled, SPELLINGS);
    for status in DraftStatus::ALL {
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            serde_json::Value::String(status.as_str().into()),
            "{status:?}"
        );
        assert_eq!(status.as_str().parse::<DraftStatus>().unwrap(), status);
    }
}

#[test]
fn every_status_round_trips_through_sqlite() {
    let conn = db();
    queries::insert_tx_draft(&conn, "d1", PROFILE, "send_hns", "00", "[]", "{}").unwrap();
    for status in DraftStatus::ALL {
        queries::update_tx_draft_status(&conn, "d1", status, None, None).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT status FROM wallet_tx_drafts WHERE id = 'd1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, status.as_str());
        let row = queries::get_tx_draft(&conn, "d1").unwrap().unwrap();
        assert_eq!(row.status, status);
        assert_eq!(row.to_summary().status, status);
    }
}

#[test]
fn an_unknown_stored_status_is_an_error_not_a_default() {
    assert!("broadcast".parse::<DraftStatus>().is_err());
    assert!("".parse::<DraftStatus>().is_err());
    assert!("Draft".parse::<DraftStatus>().is_err());

    // A row past the CHECK (it is switched off for this one insert) must not
    // read as some status: the read fails and names the spelling.
    let conn = db();
    conn.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    insert_raw(&conn, "bad", "broadcast").unwrap();
    conn.pragma_update(None, "ignore_check_constraints", false)
        .unwrap();
    let err = queries::get_tx_draft(&conn, "bad").unwrap_err();
    assert!(
        err.to_string().contains("unknown draft status 'broadcast'"),
        "{err}"
    );
}

#[test]
fn the_status_check_refuses_an_unknown_spelling() {
    let conn = db();
    for (i, spelling) in SPELLINGS.iter().enumerate() {
        insert_raw(&conn, &format!("ok{i}"), spelling).unwrap();
    }
    for bad in ["broadcast", "pending", "DRAFT", ""] {
        let err = insert_raw(&conn, &format!("bad-{bad}"), bad).unwrap_err();
        assert!(
            err.to_string().contains("CHECK constraint failed"),
            "{bad}: {err}"
        );
    }
}

/// The CHECK's list is the enum's, whole: a status added to one and not the
/// other fails here.
#[test]
fn the_status_check_lists_exactly_the_draft_statuses() {
    let conn = db();
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'wallet_tx_drafts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let start = sql.find("CHECK (status IN (").expect("status CHECK") + "CHECK (status IN (".len();
    let end = start + sql[start..].find(')').unwrap();
    let mut listed: Vec<String> = sql[start..end]
        .split(',')
        .map(|s| s.trim().trim_matches('\'').to_string())
        .collect();
    listed.sort();
    let mut known: Vec<String> = DraftStatus::ALL
        .iter()
        .map(|s| s.as_str().to_string())
        .collect();
    known.sort();
    assert_eq!(listed, known);
}
