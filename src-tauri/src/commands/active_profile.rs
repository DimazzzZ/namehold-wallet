//! The active wallet profile's `Network`, resolved once for every command that
//! needs it (node lifecycle, Namebase withdrawals). Replaces the identical
//! private copies that used to live in `commands/node.rs` and
//! `commands/namebase.rs`.

use crate::db;
use crate::noncustodial::network::Network;
use crate::AppState;

/// The active profile's network, defaulting to mainnet — matches the network
/// the rest of the app operates on (and the default RPC port). Any failure
/// (poisoned lock, DB error, unknown string) degrades to `Network::Main`.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn active_profile_network(state: &AppState) -> Network {
    let conn = match state.db.lock() {
        Ok(c) => c,
        // IO shell: mutex-poisoned branch — only reachable if a panic occurred
        // while holding the db lock. Cannot be triggered in unit tests safely.
        Err(_) => return Network::Main,
    };
    active_profile_network_from_conn(&conn)
}

/// Pure-DB core of [`active_profile_network`]: no profile, a dangling active
/// id, a DB error, or an unparseable network string all yield `Network::Main`.
pub(crate) fn active_profile_network_from_conn(conn: &rusqlite::Connection) -> Network {
    active_profile_network_opt_from_conn(conn).unwrap_or_default()
}

/// The active profile's network, or `None` when there is no profile, the active
/// id dangles, the DB errors, or the stored string does not parse.
///
/// Prefer this wherever defaulting to mainnet would be an action rather than a
/// label. Reporting "mainnet" in a status payload is harmless; *launching* a
/// mainnet node is not — it starts a full chain sync on the user's disk under a
/// data dir they set up for something else.
pub(crate) fn active_profile_network_opt_from_conn(conn: &rusqlite::Connection) -> Option<Network> {
    db::queries::get_active_profile_network(conn)
        .ok()
        .flatten()
        .and_then(|s| Network::from_str_opt(&s))
}

/// `State`-based form of [`active_profile_network_opt_from_conn`]. A poisoned
/// lock reads as "unknown" rather than mainnet, for the same reason.
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn active_profile_network_opt(state: &AppState) -> Option<Network> {
    let conn = state.db.lock().ok()?;
    active_profile_network_opt_from_conn(&conn)
}

/// The `Network` of one *named* profile, or `None` when the profile is
/// missing, the DB errors, or the stored string does not parse.
///
/// The three failures collapse into one answer on purpose: every caller of
/// this refuses to act rather than guess, and the guess would be mainnet. A
/// sync step that guessed would read another chain's explorer into this
/// profile's cache, which is the cross-network read the network guard exists
/// to prevent. Callers that only need a *label* should keep using
/// [`active_profile_network_from_conn`].
pub(crate) fn profile_network_opt_from_conn(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Option<Network> {
    db::queries::get_wallet_profile(conn, profile_id)
        .ok()
        .flatten()
        .and_then(|p| Network::from_str_opt(&p.network))
}
