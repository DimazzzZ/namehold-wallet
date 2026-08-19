//! Ledger hardware wallet support for Handshake.
//!
//! Namehold drives the official `handshake-org/ledger-app-hns` device app
//! (Nano S / Nano X, firmware 1.6.0, installed via developer mode). The wallet
//! composes the transaction locally, streams it to the device for on-screen
//! review ("parse mode"), then requests one signature per input ("sign mode").
//! Private keys never leave the device.
//!
//! # Serialization contract (source of truth)
//!
//! Byte layouts are replicated exactly from the reference client `hsd-ledger`
//! (Node.js v2.0.2) and cross-checked with `ledger-app-hns/src/apdu-signature.c`.
//! The device is strict: covenant item ordering and the appended name marker
//! must match byte-for-byte or the on-device parser rejects the transaction
//! (`0x6a80`). See [`covenant_serializer`] for the per-covenant translation
//! from Namehold's hsd-wire covenant items to the device's expected layout.
//!
//! The on-device signature preimage is a BIP143-style construction with
//! BLAKE2b-256 replacing double-SHA256 — identical to the sighash Namehold's
//! own [`crate::noncustodial::tx`] already computes. That equivalence lets us
//! verify APDU correctness offline (Speculos) against our known-good sighash.
//!
//! # Module map
//!
//! * [`apdu`] — command builders, response parsers, varint helpers.
//! * [`hid_transport`] — device discovery + 0x0101/0x05 HID framing.
//! * [`parse_mode`] — the whole-tx "parse" blob (inputs, outputs, covenants).
//! * [`sign_mode`] — the per-input "sign" request + signature parsing.
//! * [`covenant_serializer`] — hsd-wire → device covenant item translation.

pub mod apdu;
pub mod covenant_serializer;
pub mod hid_transport;
pub mod parse_mode;
pub mod sign_mode;
pub mod signing;

#[cfg(any(test, feature = "mock-ledger"))]
pub(crate) mod test_helpers;

#[cfg(feature = "mock-ledger")]
pub mod simulated_hid;

use crate::error::AppError;
use crate::noncustodial::hd::bip44_path;
use crate::noncustodial::network::Network;
use hid_transport::{HidIo, RealHid, Transport};

use apdu::network_flag;

/// Semantic version reported by the on-device Handshake app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppVersion {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

impl std::fmt::Display for AppVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A live connection to the Handshake app on a Ledger device.
///
/// Generic over the HID transport so tests (and Speculos) can inject a mock.
/// Construct a physical connection with [`LedgerSigner::connect`].
pub struct LedgerSigner<T: HidIo> {
    transport: Transport<T>,
}

/// The HID transport used by [`LedgerSigner::connect`].
///
/// In normal builds this is just [`RealHid`]. When the dev-only `mock-ledger`
/// feature is enabled, it becomes an enum that can also carry a
/// [`simulated_hid::SimulatedHid`], selected at runtime via the
/// `NAMEHOLD_LEDGER_SIM` environment variable. The simulator lets you click
/// through every Ledger UX path (reject, timeout, wrong app, disconnect, …)
/// without a physical device. It is never compiled into release builds.
#[cfg(not(feature = "mock-ledger"))]
pub type AnyHid = RealHid;

/// See [`AnyHid`] (mock-ledger build variant).
#[cfg(feature = "mock-ledger")]
pub enum AnyHid {
    Real(RealHid),
    Sim(simulated_hid::SimulatedHid),
}

#[cfg(feature = "mock-ledger")]
impl HidIo for AnyHid {
    fn write_packet(&mut self, packet: &[u8; hid_transport::PACKET_SIZE]) -> Result<(), AppError> {
        match self {
            Self::Real(r) => r.write_packet(packet),
            Self::Sim(s) => s.write_packet(packet),
        }
    }
    fn read_packet(&mut self) -> Result<[u8; hid_transport::PACKET_SIZE], AppError> {
        match self {
            Self::Real(r) => r.read_packet(),
            Self::Sim(s) => s.read_packet(),
        }
    }
}

impl LedgerSigner<AnyHid> {
    /// Open the first connected Ledger device and wrap it. Does not yet talk to
    /// the device; call [`get_app_version`](Self::get_app_version) to confirm
    /// the Handshake app is open and reachable.
    ///
    /// When built with the dev-only `mock-ledger` feature and the
    /// `NAMEHOLD_LEDGER_SIM` env var is set, this returns a simulated device
    /// instead of touching real hardware. See [`AnyHid`].
    pub fn connect() -> Result<Self, AppError> {
        #[cfg(feature = "mock-ledger")]
        {
            if let Ok(mode_str) = std::env::var("NAMEHOLD_LEDGER_SIM") {
                let mode = simulated_hid::SimMode::parse(&mode_str)?;
                if matches!(mode, simulated_hid::SimMode::NoDevice) {
                    return Err(AppError::Device(
                        "no Ledger device found — plug it in, unlock it, and open the Handshake app"
                            .into(),
                    ));
                }
                eprintln!("[mock-ledger] simulating Ledger device in '{mode_str}' mode");
                let io = AnyHid::Sim(simulated_hid::SimulatedHid::new(mode));
                return Ok(Self {
                    transport: Transport::new(io),
                });
            }
        }

        let real = RealHid::open_first()?;
        #[cfg(feature = "mock-ledger")]
        let io = AnyHid::Real(real);
        #[cfg(not(feature = "mock-ledger"))]
        let io = real;
        Ok(Self {
            transport: Transport::new(io),
        })
    }
}

impl<T: HidIo> LedgerSigner<T> {
    /// Wrap an arbitrary transport (used by tests / Speculos).
    pub fn with_transport(transport: Transport<T>) -> Self {
        Self { transport }
    }

    /// Query the running app's version. Doubles as a liveness/"is the HNS app
    /// open?" probe — a wrong app answers with a class/instruction error.
    pub fn get_app_version(&mut self) -> Result<AppVersion, AppError> {
        let body = self.transport.exchange_ok(&apdu::get_app_version())?;
        if body.len() < 3 {
            return Err(AppError::Device(format!(
                "GET_APP_VERSION returned {} bytes, expected >= 3",
                body.len()
            )));
        }
        Ok(AppVersion {
            major: body[0],
            minor: body[1],
            patch: body[2],
        })
    }

    /// Fetch the account-level extended public key from the device. Derivation
    /// path is the BIP44 account: `m/44'/coin'/account'`. The returned tuple
    /// carries the 33-byte compressed pubkey and the 32-byte chain code, ready
    /// to be assembled into a base58check-encoded xpub via
    /// [`ExtendedPubKey::to_base58check`](crate::noncustodial::hd::ExtendedPubKey::to_base58check).
    ///
    /// `confirm` — when true, the device prompts the user to approve the
    /// disclosure of the account xpub. Recommended for the first-time import.
    pub fn get_account_pubkey(
        &mut self,
        network: Network,
        account: u32,
        confirm: bool,
    ) -> Result<([u8; 33], [u8; 32]), AppError> {
        let path = account_path(network, account);
        let cmd = apdu::get_public_key(
            &path,
            confirm,
            network_flag(network),
            /* with_xpub */ true,
            /* with_address */ false,
        )?;
        let body = self.transport.exchange_ok(&cmd)?;
        let parsed = apdu::parse_public_key(&body)?;
        let chain_code = parsed
            .chain_code
            .ok_or_else(|| AppError::Device("device did not return an xpub chain code".into()))?;
        Ok((parsed.public_key, chain_code))
    }

    /// Fetch the compressed public key for a specific receive/change address.
    /// Useful for verifying an address on-device before showing it to the user.
    /// Path is `m/44'/coin'/account'/branch/index`.
    pub fn get_address_pubkey(
        &mut self,
        network: Network,
        account: u32,
        branch: u32,
        index: u32,
        confirm: bool,
    ) -> Result<[u8; 33], AppError> {
        let path = bip44_path(network, account, branch, index);
        let cmd = apdu::get_public_key(
            &path,
            confirm,
            network_flag(network),
            /* with_xpub */ false,
            /* with_address */
            confirm, // show the address on-device only when confirming
        )?;
        let body = self.transport.exchange_ok(&cmd)?;
        let parsed = apdu::parse_public_key(&body)?;
        Ok(parsed.public_key)
    }

    /// Borrow the underlying transport (parse/sign phases live in their own
    /// modules but drive the same transport).
    pub(crate) fn transport_mut(&mut self) -> &mut Transport<T> {
        &mut self.transport
    }
}

/// Build the BIP44 account-level path `m/44'/coin'/account'` for xpub export.
pub fn account_path(network: Network, account: u32) -> [u32; 3] {
    let full = bip44_path(network, account, 0, 0);
    [full[0], full[1], full[2]]
}

/// Resolve human-readable names for a Ledger-signed covenant transaction.
///
/// The Ledger app requires an [`OutputName`](parse_mode::OutputName) marker
/// for each of the 7 name-bearing covenant types (REVEAL/REDEEM/REGISTER/
/// UPDATE/RENEW/TRANSFER/REVOKE) so it can display "example/" instead of a
/// bare hash. We look up the human name by `nameHash` — always covenant
/// item[0] — in the profile's `tracked_name_states` table.
///
/// # Error handling
///
/// - JSON parse errors surface as [`AppError::Json`] (caller is likely
///   handing us a corrupted draft — refuse to sign rather than send a bogus
///   plan to the device).
/// - DB lookup errors surface as [`AppError::Db`]. Previously (H4 / M4)
///   these were silently swallowed by `if let Ok(Some(..))`, which meant a
///   transient sqlite hiccup would degrade into "device shows a bare hash"
///   with no user warning. We now propagate them.
/// - A name-bearing output whose nameHash is *not* in `tracked_name_states`
///   is skipped (returns `Ok(None)` from `get_name_by_hash`) — this is
///   legitimate: names get tracked on first sync, and a fresh import may
///   temporarily lack an entry. The parse-mode builder will fail with a
///   `Protocol` error when it needs a name it wasn't given, and that error
///   is more informative than making this function fail.
pub fn resolve_covenant_names(
    conn: &rusqlite::Connection,
    signing_inputs_json: &str,
    profile_id: &str,
) -> Result<Vec<(usize, String)>, AppError> {
    use crate::providers::ledger::covenant_serializer::requires_name_marker;
    let plan: crate::noncustodial::actions::DraftPlan =
        serde_json::from_str(signing_inputs_json)?;
    let mut names = Vec::new();
    for (i, out) in plan.outputs.iter().enumerate() {
        if !requires_name_marker(out.covenant_type) {
            continue;
        }
        // nameHash is always covenant item[0].
        let Some(hash_hex) = out.covenant_items_hex.first() else {
            continue;
        };
        // Propagate DB errors — do not swallow. A missing row is fine
        // (returns Ok(None)); a genuine failure surfaces as AppError::Db.
        if let Some(name) = crate::db::queries::get_name_by_hash(conn, profile_id, hash_hex)? {
            names.push((i, name));
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ledger::hid_transport::PACKET_SIZE;
    use std::collections::VecDeque;

    struct ScriptedHid {
        reads: VecDeque<[u8; PACKET_SIZE]>,
    }
    impl HidIo for ScriptedHid {
        fn write_packet(&mut self, _p: &[u8; PACKET_SIZE]) -> Result<(), AppError> {
            Ok(())
        }
        fn read_packet(&mut self) -> Result<[u8; PACKET_SIZE], AppError> {
            self.reads
                .pop_front()
                .ok_or_else(|| AppError::Device("no packet".into()))
        }
    }

    fn framed(body: &[u8]) -> [u8; PACKET_SIZE] {
        // Single-frame response: channel|tag|seq0|len|body|SW
        let mut raw = body.to_vec();
        raw.extend_from_slice(&0x9000u16.to_be_bytes());
        let mut pkt = [0u8; PACKET_SIZE];
        pkt[0..2].copy_from_slice(&0x0101u16.to_be_bytes());
        pkt[2] = 0x05;
        pkt[3..5].copy_from_slice(&0u16.to_be_bytes());
        pkt[5..7].copy_from_slice(&(raw.len() as u16).to_be_bytes());
        pkt[7..7 + raw.len()].copy_from_slice(&raw);
        pkt
    }

    #[test]
    fn app_version_parsed() {
        let hid = ScriptedHid {
            reads: vec![framed(&[1, 6, 0])].into(),
        };
        let mut signer = LedgerSigner::with_transport(Transport::new(hid));
        let v = signer.get_app_version().unwrap();
        assert_eq!(
            v,
            AppVersion {
                major: 1,
                minor: 6,
                patch: 0
            }
        );
        assert_eq!(v.to_string(), "1.6.0");
    }

    #[test]
    fn account_path_shape() {
        let p = account_path(Network::Main, 0);
        assert_eq!(p[0], 44 + 0x8000_0000);
        assert_eq!(p[1], 5353 + 0x8000_0000);
        assert_eq!(p[2], 0x8000_0000);
    }

    // --- M4: resolve_covenant_names behaviour ---

    /// Minimal in-memory schema sufficient for `get_name_by_hash` — just the
    /// one table it queries, without the FK to wallet_profiles (irrelevant to
    /// the lookup path we're exercising).
    fn name_states_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tracked_name_states (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 wallet_profile_id TEXT NOT NULL,
                 name TEXT NOT NULL,
                 name_hash_hex TEXT NOT NULL,
                 state TEXT NOT NULL
             );",
        )
        .unwrap();
        conn
    }

    fn insert_name(conn: &rusqlite::Connection, profile: &str, hash_hex: &str, name: &str) {
        conn.execute(
            "INSERT INTO tracked_name_states (wallet_profile_id, name, name_hash_hex, state)
             VALUES (?1, ?2, ?3, 'CLOSED')",
            rusqlite::params![profile, name, hash_hex],
        )
        .unwrap();
    }

    /// Build a signing_inputs_json string for a plan with a single covenant
    /// output of the given type, whose item[0] is `name_hash_hex`.
    fn covenant_plan_json(covenant_type: u8, name_hash_hex: &str) -> String {
        let plan = crate::noncustodial::actions::DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "main".into(),
            inputs: vec![crate::noncustodial::actions::PlanInput {
                txid: "aa".repeat(32),
                vout: 0,
                value: 100_000_000,
                branch: 0,
                child_index: 0,
                sighash_type: 1,
            }],
            outputs: vec![crate::noncustodial::actions::PlanOutput {
                value: 99_000_000,
                address: "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx".into(),
                covenant_type,
                covenant_items_hex: vec![name_hash_hex.to_string()],
            }],
            change_output_index: None,
        };
        serde_json::to_string(&plan).unwrap()
    }

    #[test]
    fn resolve_covenant_names_resolves_name_bearing_output() {
        use crate::noncustodial::sync::COV_TRANSFER; // name-bearing
        let conn = name_states_db();
        let hash = "bb".repeat(32);
        insert_name(&conn, "profile-1", &hash, "example");
        let json = covenant_plan_json(COV_TRANSFER, &hash);

        let names = resolve_covenant_names(&conn, &json, "profile-1").unwrap();
        assert_eq!(names, vec![(0usize, "example".to_string())]);
    }

    #[test]
    fn resolve_covenant_names_skips_non_name_bearing() {
        // NONE is not a name-bearing covenant → no lookup, empty result.
        let conn = name_states_db();
        let json = covenant_plan_json(crate::noncustodial::sync::COV_NONE, &"bb".repeat(32));
        let names = resolve_covenant_names(&conn, &json, "profile-1").unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn resolve_covenant_names_skips_untracked_hash() {
        // A name-bearing covenant whose hash isn't tracked yet → skipped, not
        // an error (get_name_by_hash returns Ok(None)).
        use crate::noncustodial::sync::COV_TRANSFER;
        let conn = name_states_db();
        let json = covenant_plan_json(COV_TRANSFER, &"cc".repeat(32));
        let names = resolve_covenant_names(&conn, &json, "profile-1").unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn resolve_covenant_names_propagates_json_error() {
        let conn = name_states_db();
        let err = resolve_covenant_names(&conn, "{ not valid json", "profile-1").unwrap_err();
        assert!(
            matches!(err, AppError::Json(_)),
            "malformed draft JSON should surface as AppError::Json, got: {err:?}"
        );
    }

    #[test]
    fn resolve_covenant_names_propagates_db_error() {
        // Drop the table so the query fails — the error must propagate (M4:
        // previously an `if let Ok(..)` swallowed it into a silent skip).
        let conn = name_states_db();
        conn.execute_batch("DROP TABLE tracked_name_states;").unwrap();
        let json = covenant_plan_json(crate::noncustodial::sync::COV_TRANSFER, &"bb".repeat(32));
        let err = resolve_covenant_names(&conn, &json, "profile-1").unwrap_err();
        assert!(
            matches!(err, AppError::Db(_)),
            "a DB failure must propagate as AppError::Db, got: {err:?}"
        );
    }
}
