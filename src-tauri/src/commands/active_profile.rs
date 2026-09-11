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
    db::queries::get_active_profile_network(conn)
        .ok()
        .flatten()
        .and_then(|s| Network::from_str_opt(&s))
        .unwrap_or_default()
}
