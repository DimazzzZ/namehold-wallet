//! Watch-only address derivation and persistence for non-custodial profiles.
//!
//! Every profile stores an account-level extended public key (`account_xpub`,
//! i.e. the BIP44 node `m/44'/coin'/account'`). All receive/change addresses
//! are derived *publicly* from that xpub — no secret material is required, so
//! this works identically for hot and watch-only profiles. This is the BIP32
//! invariant proven in `hd.rs`: public derivation of a non-hardened child
//! yields the same key as private derivation.
//!
//! BIP44 layout below the account node:
//!   - branch 0 = receive (external) chain
//!   - branch 1 = change   (internal) chain
//!
//! Derived addresses are persisted to the `derived_addresses` table so the
//! sync engine can match node UTXOs back to a `(profile, branch, index)`
//! without re-deriving on every scan.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::hd::ExtendedPubKey;
use crate::noncustodial::network::Network;

/// BIP44 external (receive) chain.
pub const BRANCH_RECEIVE: u32 = 0;
/// BIP44 internal (change) chain.
pub const BRANCH_CHANGE: u32 = 1;

/// A single derived address with everything the sync engine and UI need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedAddress {
    pub branch: u32,
    pub child_index: u32,
    pub address: String,
    pub script_pubkey_hex: String,
    pub public_key_hex: String,
}

/// Map a stored profile `network` value to a [`Network`].
///
/// Migration 006 stores `'mainnet' | 'testnet' | 'regtest'`; `Network`'s
/// parser also accepts `'main'`, so both spellings resolve.
pub fn network_from_profile(value: &str) -> Result<Network, AppError> {
    Network::from_str_opt(value)
        .ok_or_else(|| AppError::InvalidInput(format!("unknown profile network '{value}'")))
}

/// Derive a single address from an account-level xpub.
///
/// `branch` must be 0 (receive) or 1 (change); `index` is the non-hardened
/// child index on that branch.
pub fn derive_one(
    network: Network,
    account_xpub: &ExtendedPubKey,
    branch: u32,
    index: u32,
) -> Result<DerivedAddress, AppError> {
    if branch != BRANCH_RECEIVE && branch != BRANCH_CHANGE {
        return Err(AppError::InvalidInput(format!(
            "branch must be 0 (receive) or 1 (change), got {branch}"
        )));
    }
    let child = account_xpub.derive_path(&[branch, index])?;
    let pubkey = child.compressed_pubkey();
    let addr = address::address_from_pubkey(network, &pubkey)?;
    let script = address::script_pubkey_from_pubkey(&pubkey)?;
    Ok(DerivedAddress {
        branch,
        child_index: index,
        address: addr,
        script_pubkey_hex: hex::encode(script),
        public_key_hex: hex::encode(pubkey),
    })
}

/// Derive a contiguous range `[start, start+count)` of addresses on a branch.
pub fn derive_range(
    network: Network,
    account_xpub: &ExtendedPubKey,
    branch: u32,
    start: u32,
    count: u32,
) -> Result<Vec<DerivedAddress>, AppError> {
    let mut out = Vec::with_capacity(count as usize);
    for offset in 0..count {
        let index = start
            .checked_add(offset)
            .ok_or_else(|| AppError::InvalidInput("child index overflow".to_string()))?;
        out.push(derive_one(network, account_xpub, branch, index)?);
    }
    Ok(out)
}

/// Insert (or ignore if already present) a derived address for a profile.
///
/// Idempotent: the `derived_addresses` UNIQUE(profile, account, branch, index)
/// constraint makes re-deriving the same slot a no-op. Returns whether a new
/// row was inserted.
pub fn persist_address(
    conn: &Connection,
    profile_id: &str,
    account_index: u32,
    derived: &DerivedAddress,
) -> Result<bool, AppError> {
    let changed = conn.execute(
        "INSERT OR IGNORE INTO derived_addresses
            (wallet_profile_id, account_index, branch, child_index,
             address, script_pubkey_hex, public_key_hex)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            profile_id,
            account_index as i64,
            derived.branch as i64,
            derived.child_index as i64,
            derived.address,
            derived.script_pubkey_hex,
            derived.public_key_hex,
        ],
    )?;
    Ok(changed > 0)
}

/// The highest derived `child_index` for a profile+branch, or `None` if no
/// address has been derived on that branch yet.
pub fn max_derived_index(
    conn: &Connection,
    profile_id: &str,
    account_index: u32,
    branch: u32,
) -> Result<Option<u32>, AppError> {
    let max: Option<i64> = conn
        .query_row(
            "SELECT MAX(child_index) FROM derived_addresses
             WHERE wallet_profile_id = ?1 AND account_index = ?2 AND branch = ?3",
            params![profile_id, account_index as i64, branch as i64],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    Ok(max.map(|m| m as u32))
}

/// Ensure at least `count` addresses exist on a branch, deriving and persisting
/// any that are missing. Returns the full contiguous list `[0, count)`.
///
/// This is the gap-limit primitive: callers pass `used_count + gap_limit` to
/// guarantee a lookahead window of unused addresses.
pub fn ensure_addresses(
    conn: &Connection,
    profile_id: &str,
    account_index: u32,
    network: Network,
    account_xpub: &ExtendedPubKey,
    branch: u32,
    count: u32,
) -> Result<Vec<DerivedAddress>, AppError> {
    let derived = derive_range(network, account_xpub, branch, 0, count)?;
    for d in &derived {
        persist_address(conn, profile_id, account_index, d)?;
    }
    Ok(derived)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::hd::{seed_from_mnemonic, ExtendedPrivKey};

    fn test_xpub() -> ExtendedPubKey {
        let seed = seed_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "",
        )
        .unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).unwrap();
        // Use the master node directly as the "account xpub" for test purposes;
        // derivation semantics (branch/index) are identical regardless of depth.
        ExtendedPubKey::from_priv(&master)
    }

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../sql/006_noncustodial_wallet_profiles.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/007_noncustodial_chain_cache.sql"))
            .unwrap();
        // Insert a profile to satisfy the FK on derived_addresses.
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_xpub)
             VALUES ('p1', 'Test', 'watch_only_xpub', 'mainnet', 'xpubPLACEHOLDER')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn network_from_profile_accepts_both_spellings() {
        assert_eq!(network_from_profile("mainnet").unwrap(), Network::Main);
        assert_eq!(network_from_profile("main").unwrap(), Network::Main);
        assert_eq!(network_from_profile("testnet").unwrap(), Network::Testnet);
        assert!(network_from_profile("bogus").is_err());
    }

    #[test]
    fn derive_one_is_deterministic_and_valid() {
        let xpub = test_xpub();
        let a = derive_one(Network::Main, &xpub, BRANCH_RECEIVE, 0).unwrap();
        let b = derive_one(Network::Main, &xpub, BRANCH_RECEIVE, 0).unwrap();
        assert_eq!(a, b);
        assert!(a.address.starts_with("hs1"));
        assert!(address::is_valid(Network::Main, &a.address));
        // script_pubkey is 22-byte P2WPKH: 0014 + 20-byte hash.
        assert!(a.script_pubkey_hex.starts_with("0014"));
        assert_eq!(a.script_pubkey_hex.len(), 44);
        assert_eq!(a.public_key_hex.len(), 66);
    }

    #[test]
    fn receive_and_change_branches_differ() {
        let xpub = test_xpub();
        let recv = derive_one(Network::Main, &xpub, BRANCH_RECEIVE, 0).unwrap();
        let change = derive_one(Network::Main, &xpub, BRANCH_CHANGE, 0).unwrap();
        assert_ne!(recv.address, change.address);
    }

    #[test]
    fn derive_one_rejects_bad_branch() {
        let xpub = test_xpub();
        assert!(derive_one(Network::Main, &xpub, 2, 0).is_err());
    }

    #[test]
    fn derive_range_is_contiguous() {
        let xpub = test_xpub();
        let range = derive_range(Network::Main, &xpub, BRANCH_RECEIVE, 0, 5).unwrap();
        assert_eq!(range.len(), 5);
        for (i, d) in range.iter().enumerate() {
            assert_eq!(d.child_index, i as u32);
            assert_eq!(d.branch, BRANCH_RECEIVE);
        }
        // All addresses are distinct.
        let mut addrs: Vec<_> = range.iter().map(|d| d.address.clone()).collect();
        addrs.sort();
        addrs.dedup();
        assert_eq!(addrs.len(), 5);
    }

    #[test]
    fn persist_is_idempotent_and_tracks_max_index() {
        let conn = mem_db();
        let xpub = test_xpub();

        assert_eq!(
            max_derived_index(&conn, "p1", 0, BRANCH_RECEIVE).unwrap(),
            None
        );

        let d0 = derive_one(Network::Main, &xpub, BRANCH_RECEIVE, 0).unwrap();
        let d1 = derive_one(Network::Main, &xpub, BRANCH_RECEIVE, 1).unwrap();
        assert!(persist_address(&conn, "p1", 0, &d0).unwrap());
        assert!(persist_address(&conn, "p1", 0, &d1).unwrap());
        // Re-persisting the same slot is a no-op.
        assert!(!persist_address(&conn, "p1", 0, &d0).unwrap());

        assert_eq!(
            max_derived_index(&conn, "p1", 0, BRANCH_RECEIVE).unwrap(),
            Some(1)
        );

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM derived_addresses WHERE wallet_profile_id = 'p1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn ensure_addresses_fills_window() {
        let conn = mem_db();
        let xpub = test_xpub();
        let list = ensure_addresses(&conn, "p1", 0, Network::Main, &xpub, BRANCH_RECEIVE, 10)
            .unwrap();
        assert_eq!(list.len(), 10);
        assert_eq!(
            max_derived_index(&conn, "p1", 0, BRANCH_RECEIVE).unwrap(),
            Some(9)
        );

        // Calling again with the same count re-derives but inserts nothing new.
        let again = ensure_addresses(&conn, "p1", 0, Network::Main, &xpub, BRANCH_RECEIVE, 10)
            .unwrap();
        assert_eq!(again, list);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM derived_addresses WHERE branch = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 10);
    }
}
