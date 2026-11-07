//! The relay's own SQLite database (not the wallet's `portfolio.db`): offers,
//! messages and watch jobs. Everything in it was verified before it was
//! stored (`handlers`), and nothing in it can move a coin: the offers and
//! messages are signed by relay keys, the jobs hold only transactions that
//! pay the party that signed them.
//!
//! The functions take a `&Connection`; [`Store`] holds the one connection
//! the server shares between requests and the watchtower. No lock is held
//! across an `.await`: the watchtower reads its jobs, releases the lock,
//! talks to the chains, then writes.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::AppError;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS offers (
    id           TEXT PRIMARY KEY,
    network      TEXT NOT NULL,
    maker_key    TEXT NOT NULL,
    json         TEXT NOT NULL,
    -- When this server first stored it (the order of the list).
    created_at   INTEGER NOT NULL,
    last_seen    INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    -- Set by the maker's withdrawal: the offer is gone, and a re-post of
    -- the same signed offer is refused until it expires.
    withdrawn_at INTEGER
);
CREATE INDEX IF NOT EXISTS offers_by_network ON offers (network, last_seen);
CREATE INDEX IF NOT EXISTS offers_by_maker ON offers (maker_key);

CREATE TABLE IF NOT EXISTS messages (
    -- AUTOINCREMENT: never reused, even after the newest rows are purged,
    -- so a cursor never skips a message stored later.
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    id           TEXT NOT NULL UNIQUE,
    to_key       TEXT NOT NULL,
    from_key     TEXT NOT NULL,
    json         TEXT NOT NULL,
    received_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_by_to ON messages (to_key, seq);
CREATE INDEX IF NOT EXISTS messages_by_from ON messages (from_key, received_at);

CREATE TABLE IF NOT EXISTS watch_jobs (
    id           TEXT PRIMARY KEY,
    network      TEXT NOT NULL,
    json         TEXT NOT NULL,
    status_json  TEXT NOT NULL,
    sent_json    TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    -- When the job's status first became final (completed, refunded,
    -- expired); cleared if a later step finds otherwise.
    finished_at  INTEGER,
    last_error   TEXT
);
PRAGMA user_version = 1;
";

/// The server's database.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (or create) the database file at `path`, in WAL mode, readable
    /// by its owner only. A missing directory is created owner-only; an
    /// existing one is left as it is (it may be `/var/lib`).
    pub fn open(path: &Path) -> Result<Store, AppError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            if !parent.exists() {
                crate::db::connection::secure_data_dir(parent)?;
            }
        }
        // Owner-only before SQLite opens it: SQLite gives the -wal and -shm
        // files the database file's mode when it creates them.
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        let conn = Connection::open(path)?;
        let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(AppError::Other(format!(
                "the relay database did not switch to WAL (it is '{mode}')"
            )));
        }
        Store::init(conn)
    }

    /// A database in memory, for tests.
    pub fn open_in_memory() -> Result<Store, AppError> {
        Store::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store, AppError> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    /// The connection, for one short piece of work.
    pub fn lock(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.conn
            .lock()
            .map_err(|_| AppError::Lock("the relay database lock is poisoned".into()))
    }
}

/// SQLite integers are signed. Times and counts here are Unix seconds and
/// row numbers, far below 2^63; a larger value (a forged `expiresAt`) is
/// stored as the largest, which keeps every comparison's direction.
fn int(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn col(r: &Row<'_>, i: usize) -> rusqlite::Result<u64> {
    let v: i64 = r.get(i)?;
    u64::try_from(v).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(i, v))
}

fn col_opt(r: &Row<'_>, i: usize) -> rusqlite::Result<Option<u64>> {
    let v: Option<i64> = r.get(i)?;
    v.map(|v| u64::try_from(v).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(i, v)))
        .transpose()
}

fn count(c: &Connection, sql: &str, p: impl rusqlite::Params) -> Result<u64, AppError> {
    Ok(c.query_row(sql, p, |r| col(r, 0))?)
}

// ------------------------------------------------------------------ offers

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferRow {
    pub id: String,
    pub network: String,
    pub maker_key: String,
    pub json: String,
    pub created_at: u64,
    pub last_seen: u64,
    pub expires_at: u64,
    pub withdrawn_at: Option<u64>,
}

fn offer_row(r: &Row<'_>) -> rusqlite::Result<OfferRow> {
    Ok(OfferRow {
        id: r.get(0)?,
        network: r.get(1)?,
        maker_key: r.get(2)?,
        json: r.get(3)?,
        created_at: col(r, 4)?,
        last_seen: col(r, 5)?,
        expires_at: col(r, 6)?,
        withdrawn_at: col_opt(r, 7)?,
    })
}

const OFFER_COLS: &str =
    "id, network, maker_key, json, created_at, last_seen, expires_at, withdrawn_at";

pub fn get_offer(c: &Connection, id: &str) -> Result<Option<OfferRow>, AppError> {
    Ok(c.query_row(
        &format!("SELECT {OFFER_COLS} FROM offers WHERE id = ?1"),
        [id],
        offer_row,
    )
    .optional()?)
}

pub fn insert_offer(c: &Connection, o: &OfferRow) -> Result<(), AppError> {
    c.execute(
        "INSERT INTO offers (id, network, maker_key, json, created_at, last_seen, expires_at, withdrawn_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            o.id,
            o.network,
            o.maker_key,
            o.json,
            int(o.created_at),
            int(o.last_seen),
            int(o.expires_at),
            o.withdrawn_at.map(int)
        ],
    )?;
    Ok(())
}

pub fn touch_offer(c: &Connection, id: &str, now: u64) -> Result<(), AppError> {
    c.execute(
        "UPDATE offers SET last_seen = ?2 WHERE id = ?1",
        params![id, int(now)],
    )?;
    Ok(())
}

pub fn withdraw_offer(c: &Connection, id: &str, now: u64) -> Result<(), AppError> {
    c.execute(
        "UPDATE offers SET withdrawn_at = COALESCE(withdrawn_at, ?2) WHERE id = ?1",
        params![id, int(now)],
    )?;
    Ok(())
}

/// The SQL condition of a live offer at `?1`: not withdrawn, not expired,
/// seen within `?2` seconds.
const LIVE: &str = "withdrawn_at IS NULL AND expires_at > ?1 AND last_seen + ?2 >= ?1";

pub fn count_live_by_maker(
    c: &Connection,
    maker_key: &str,
    now: u64,
    window: u64,
) -> Result<u64, AppError> {
    count(
        c,
        &format!("SELECT COUNT(*) FROM offers WHERE {LIVE} AND maker_key = ?3"),
        params![int(now), int(window), maker_key],
    )
}

/// Whether offer `id` is live at `now`.
pub fn offer_is_live(c: &Connection, id: &str, now: u64, window: u64) -> Result<bool, AppError> {
    let n = count(
        c,
        &format!("SELECT COUNT(*) FROM offers WHERE {LIVE} AND id = ?3"),
        params![int(now), int(window), id],
    )?;
    Ok(n > 0)
}

/// Live offers of `network`, newest first (by when this server first
/// stored them), at most `limit`.
pub fn live_offers(
    c: &Connection,
    network: &str,
    now: u64,
    window: u64,
    limit: u64,
) -> Result<Vec<OfferRow>, AppError> {
    let mut st = c.prepare(&format!(
        "SELECT {OFFER_COLS} FROM offers WHERE {LIVE} AND network = ?3
         ORDER BY created_at DESC, rowid DESC LIMIT ?4"
    ))?;
    let rows = st
        .query_map(
            params![int(now), int(window), network, int(limit)],
            offer_row,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------- messages

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    pub seq: u64,
    pub id: String,
    pub json: String,
}

pub fn message_exists(c: &Connection, id: &str) -> Result<bool, AppError> {
    Ok(
        c.query_row("SELECT 1 FROM messages WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some(),
    )
}

/// Store a message; its sequence number (the cursor after it).
pub fn insert_message(
    c: &Connection,
    id: &str,
    to_key: &str,
    from_key: &str,
    json: &str,
    received_at: u64,
) -> Result<u64, AppError> {
    c.execute(
        "INSERT INTO messages (id, to_key, from_key, json, received_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, to_key, from_key, json, int(received_at)],
    )?;
    u64::try_from(c.last_insert_rowid())
        .map_err(|_| AppError::Other("a message row id is negative".into()))
}

/// Messages from `from_key` received after `since`.
pub fn count_from_since(c: &Connection, from_key: &str, since: u64) -> Result<u64, AppError> {
    count(
        c,
        "SELECT COUNT(*) FROM messages WHERE from_key = ?1 AND received_at > ?2",
        params![from_key, int(since)],
    )
}

/// Messages to `to_key` stored after cursor `after` and received after
/// `since`, in arrival order, at most `limit`.
pub fn messages_after(
    c: &Connection,
    to_key: &str,
    after: u64,
    since: u64,
    limit: u64,
) -> Result<Vec<MessageRow>, AppError> {
    let mut st = c.prepare(
        "SELECT seq, id, json FROM messages
         WHERE to_key = ?1 AND seq > ?2 AND received_at > ?3
         ORDER BY seq ASC LIMIT ?4",
    )?;
    let rows = st
        .query_map(params![to_key, int(after), int(since), int(limit)], |r| {
            Ok(MessageRow {
                seq: col(r, 0)?,
                id: r.get(1)?,
                json: r.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// -------------------------------------------------------------- watch jobs

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    pub id: String,
    pub network: String,
    pub json: String,
    pub status_json: String,
    pub sent_json: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub finished_at: Option<u64>,
    pub last_error: Option<String>,
}

const JOB_COLS: &str =
    "id, network, json, status_json, sent_json, created_at, updated_at, finished_at, last_error";

fn job_row(r: &Row<'_>) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        id: r.get(0)?,
        network: r.get(1)?,
        json: r.get(2)?,
        status_json: r.get(3)?,
        sent_json: r.get(4)?,
        created_at: col(r, 5)?,
        updated_at: col(r, 6)?,
        finished_at: col_opt(r, 7)?,
        last_error: r.get(8)?,
    })
}

pub fn get_job(c: &Connection, id: &str) -> Result<Option<JobRow>, AppError> {
    Ok(c.query_row(
        &format!("SELECT {JOB_COLS} FROM watch_jobs WHERE id = ?1"),
        [id],
        job_row,
    )
    .optional()?)
}

pub fn insert_job(c: &Connection, j: &JobRow) -> Result<(), AppError> {
    c.execute(
        "INSERT INTO watch_jobs (id, network, json, status_json, sent_json, created_at, updated_at, finished_at, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            j.id,
            j.network,
            j.json,
            j.status_json,
            j.sent_json,
            int(j.created_at),
            int(j.updated_at),
            j.finished_at.map(int),
            j.last_error
        ],
    )?;
    Ok(())
}

/// The jobs the watchtower still runs: not finished, or finished after
/// `finished_after` (a final status is re-checked for a while, in case a
/// chain reorganises or a late transaction appears).
pub fn jobs_to_run(c: &Connection, finished_after: u64) -> Result<Vec<JobRow>, AppError> {
    let mut st = c.prepare(&format!(
        "SELECT {JOB_COLS} FROM watch_jobs
         WHERE finished_at IS NULL OR finished_at > ?1
         ORDER BY created_at ASC, id ASC"
    ))?;
    let rows = st
        .query_map([int(finished_after)], job_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// What one watchtower step leaves on a job.
pub struct JobUpdate<'a> {
    /// `None`: the step decided nothing; the last status stays.
    pub status_json: Option<&'a str>,
    pub sent_json: &'a str,
    /// The status is final: `finished_at` is set if it was not.
    pub finished: bool,
    pub last_error: Option<&'a str>,
}

pub fn save_job_step(
    c: &Connection,
    id: &str,
    u: &JobUpdate<'_>,
    now: u64,
) -> Result<(), AppError> {
    c.execute(
        "UPDATE watch_jobs SET
            status_json = COALESCE(?2, status_json),
            sent_json = ?3,
            updated_at = ?4,
            finished_at = CASE WHEN ?5 THEN COALESCE(finished_at, ?4) ELSE NULL END,
            last_error = ?6
         WHERE id = ?1",
        params![
            id,
            u.status_json,
            u.sent_json,
            int(now),
            u.finished,
            u.last_error
        ],
    )?;
    Ok(())
}

// ------------------------------------------------------------------- purge

/// What a purge removed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Purged {
    pub offers: usize,
    pub messages: usize,
    pub jobs: usize,
}

/// Remove expired offers, offers silent since `silent_before` (withdrawn
/// ones are kept until they expire, so they cannot be re-posted), messages
/// received before `messages_before`, and jobs finished before
/// `jobs_finished_before`.
pub fn purge(
    c: &Connection,
    now: u64,
    silent_before: u64,
    messages_before: u64,
    jobs_finished_before: u64,
) -> Result<Purged, AppError> {
    let offers = c.execute(
        "DELETE FROM offers WHERE expires_at <= ?1 OR (withdrawn_at IS NULL AND last_seen < ?2)",
        params![int(now), int(silent_before)],
    )?;
    let messages = c.execute(
        "DELETE FROM messages WHERE received_at < ?1",
        [int(messages_before)],
    )?;
    let jobs = c.execute(
        "DELETE FROM watch_jobs WHERE finished_at IS NOT NULL AND finished_at < ?1",
        [int(jobs_finished_before)],
    )?;
    Ok(Purged {
        offers,
        messages,
        jobs,
    })
}
