//! Standard (plain) HNS send: coin selection, fee estimation, change handling,
//! and signing for the non-custodial engine.
//!
//! This module turns a high-level intent ("send N dollarydoos to address A")
//! into a fully-signed [`Transaction`] ready for `sendrawtransaction`. It is
//! deliberately scoped to *plain* sends — outputs carry an empty covenant
//! (`00 00`). Name covenants (OPEN/BID/REVEAL/…) are handled elsewhere.
//!
//! Pipeline:
//!   1. Load the profile's spendable coins by joining `tracked_utxos` to
//!      `derived_addresses` (unspent, covenant-free, liquid P2WPKH).
//!   2. Select coins to cover `amount + fee` using a deterministic
//!      largest-first strategy, recomputing the fee as inputs are added.
//!   3. Build the recipient output plus a change output (dropped if it would
//!      be dust). The fee is the implicit remainder `sum(inputs) - outputs`.
//!   4. Re-derive each input's signing key from the unlocked session and sign
//!      with `sign_p2wpkh_input` (P2WPKH, SIGHASH_ALL).
//!
//! Fee policy is verified against hsrd `lib/protocol/policy.js`:
//!   - `MIN_RELAY = 1000` dollarydoos per 1000 bytes (1 dood/byte floor).
//!   - Dust is computed from the output size at the min relay rate; for a
//!     standard 31-byte P2WPKH output hsrd's threshold works out well under
//!     `DUST_THRESHOLD`. We use a conservative fixed dust floor below.

use rusqlite::{params, Connection};

use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::network::Network;
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::tx::{
    output_address_from_string, sighash, Covenant, Input, Outpoint, Output, Transaction,
};

/// Minimum relay fee rate in dollarydoos per byte (hsrd `MIN_RELAY` is 1000
/// dollarydoos per 1000 bytes = 1 dollarydoo/byte).
pub const MIN_FEE_RATE_PER_BYTE: u64 = 1;

/// Default fee rate used when the caller does not specify one. A small multiple
/// of the relay floor to land in a mined block promptly without overpaying.
pub const DEFAULT_FEE_RATE_PER_BYTE: u64 = 1;

/// Dust threshold in dollarydoos. Change below this is dropped into the fee
/// rather than created as an unspendable-in-practice output. A standard
/// P2WPKH output costs ~31 bytes to create and ~41 to later spend; at the
/// relay floor that is well under 1000, so we use a conservative round floor.
pub const DUST_THRESHOLD: u64 = 1000;

/// How long a coin reservation (`tracked_utxos.reserved_by_draft_id`) stays
/// valid before it is considered abandoned (I3). There is no background
/// sweeper: expiry is enforced lazily, opportunistically, wherever
/// [`load_spendable_coins`] runs — a reservation older than this is ignored
/// for selection purposes AND cleared so it doesn't need to be re-checked.
/// One hour comfortably covers a user reviewing a preview before signing,
/// while still recovering promptly from a crashed/abandoned build.
pub const RESERVATION_TTL_SECS: i64 = 3600;

/// Serialized size (bytes) of one P2WPKH input *including* its witness.
///
/// Non-witness part: outpoint(36) + sequence(4) = 40 bytes.
/// Witness part: varint(2) + varbytes(sig 65 -> 1+65) + varbytes(pubkey 33 ->
/// 1+33) = 1 + 66 + 34 = 101 bytes.
/// Total per input = 141 bytes. Handshake has no witness discount, so every
/// byte counts at the same rate.
pub const INPUT_VBYTES: u64 = 141;

/// Serialized size (bytes) of one P2WPKH output.
///
/// value(8) + address(version 1 + len 1 + program 20 = 22) + covenant(type 1 +
/// count 1 = 2) = 32 bytes.
pub const OUTPUT_VBYTES: u64 = 32;

/// Fixed transaction overhead (bytes): version(4) + locktime(4) + the two
/// varints for input and output counts (1 each for small txs). = 10 bytes.
pub const TX_OVERHEAD_VBYTES: u64 = 10;

/// A coin the wallet can spend, loaded by joining `tracked_utxos` to
/// `derived_addresses` (see [`load_spendable_coins`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendableCoin {
    /// Funding transaction id in hsrd natural-order hex (as stored / as the node
    /// reports it; Handshake does not byte-reverse hashes).
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    /// BIP44 branch (0 receive / 1 change) the paying address lives on.
    pub branch: u32,
    /// BIP44 child index of the paying address.
    pub child_index: u32,
}

/// Estimated transaction size in bytes for `n_inputs` P2WPKH inputs and
/// `n_outputs` P2WPKH outputs.
pub fn estimate_size(n_inputs: u64, n_outputs: u64) -> u64 {
    TX_OVERHEAD_VBYTES + n_inputs * INPUT_VBYTES + n_outputs * OUTPUT_VBYTES
}

/// Fee in dollarydoos for a tx of the given input/output counts at `rate`
/// (dollarydoos per byte).
pub fn estimate_fee(n_inputs: u64, n_outputs: u64, rate_per_byte: u64) -> u64 {
    estimate_size(n_inputs, n_outputs).saturating_mul(rate_per_byte.max(MIN_FEE_RATE_PER_BYTE))
}

/// Estimated transaction size in bytes for `n_inputs` P2WPKH inputs, ONE
/// "primary" output of `primary_vbytes` bytes, and `n_plain_outputs` flat
/// P2WPKH outputs (typically a 0/1 change output).
///
/// `primary_vbytes` is the output's REAL serialized length (see
/// [`crate::noncustodial::tx::Output::encoded_len`]), not the flat
/// [`OUTPUT_VBYTES`] approximation — covenant items (REGISTER/UPDATE
/// resource records, FINALIZE, …) can make a covenant output far larger than
/// a plain P2WPKH output (I4). Handshake outputs carry no witness data, so
/// every covenant byte counts fully toward vsize; there is no discount to
/// apply here.
pub fn estimate_size_with_primary(n_inputs: u64, primary_vbytes: u64, n_plain_outputs: u64) -> u64 {
    TX_OVERHEAD_VBYTES + n_inputs * INPUT_VBYTES + primary_vbytes + n_plain_outputs * OUTPUT_VBYTES
}

/// Fee in dollarydoos for [`estimate_size_with_primary`] at `rate`
/// (dollarydoos per byte, floored at [`MIN_FEE_RATE_PER_BYTE`]).
pub fn estimate_fee_with_primary(
    n_inputs: u64,
    primary_vbytes: u64,
    n_plain_outputs: u64,
    rate_per_byte: u64,
) -> u64 {
    estimate_size_with_primary(n_inputs, primary_vbytes, n_plain_outputs)
        .saturating_mul(rate_per_byte.max(MIN_FEE_RATE_PER_BYTE))
}

/// Release stale `reserved_by_draft_id` claims for a profile (I3), so an
/// abandoned build doesn't lock its coins out of future selection forever.
/// Two cases are cleared:
///   1. Dangling — the claiming draft row no longer exists (e.g. a delete
///      that, for whatever reason, didn't clear the reservation itself).
///   2. Expired — the claiming draft is older than [`RESERVATION_TTL_SECS`]
///      AND has not reached (or possibly reached) the chain (`status` is not
///      `broadcasted` / `confirmed` / `broadcast_pending`). There's no
///      separate "reserved at" timestamp; the draft's own `created_at`
///      doubles as the reservation age, and `created_at` is never refreshed
///      on broadcast — a draft signed and broadcast within the review window
///      can still cross the TTL while its inputs are genuinely in flight
///      (accepted to the node's mempool, but `tracked_utxos.spent_by_txid`
///      not yet set — that only happens on the next `sync_wallet_state`,
///      which can be minutes away). Without this status guard the TTL sweep
///      would free that in-flight coin for re-selection by another draft —
///      the node would reject the resulting double-spend, but the
///      reservation invariant (each unspent coin claimed by at most one live
///      draft) would already be violated for the window in between.
///      `broadcast_pending` (a transport-ambiguous broadcast attempt — see
///      `commands::tx::broadcast_tx_draft`) gets the same exclusion: the tx
///      may already be sitting in the node's mempool even though we never
///      got a definitive answer. `broadcasted`/`confirmed`/`broadcast_pending`
///      drafts release their reservation through their own status
///      transitions instead (broadcast rejection, or
///      `refresh_tx_confirmations` judging the tx `dropped`), never through
///      the TTL.
///
/// Called opportunistically wherever coin selection reads `tracked_utxos`
/// (see [`load_spendable_coins`]) rather than from a background job. Returns
/// the number of coins released.
pub fn release_stale_reservations(conn: &Connection, profile_id: &str) -> Result<usize, AppError> {
    let dangling = conn.execute(
        "UPDATE tracked_utxos SET reserved_by_draft_id = NULL
         WHERE wallet_profile_id = ?1
           AND reserved_by_draft_id IS NOT NULL
           AND reserved_by_draft_id NOT IN (SELECT id FROM wallet_tx_drafts)",
        params![profile_id],
    )?;
    let expired = conn.execute(
        &format!(
            "UPDATE tracked_utxos SET reserved_by_draft_id = NULL
             WHERE wallet_profile_id = ?1
               AND reserved_by_draft_id IS NOT NULL
               AND reserved_by_draft_id IN (
                   SELECT id FROM wallet_tx_drafts
                   WHERE created_at < datetime('now', '-{RESERVATION_TTL_SECS} seconds')
                     AND status NOT IN ('broadcasted', 'confirmed', 'broadcast_pending')
               )"
        ),
        params![profile_id],
    )?;
    Ok(dangling + expired)
}

/// Load ONLY the still-unspent liquid coins reserved by `draft_id` (I3),
/// largest-first — the exact set `insert_tx_draft_reserving_coins` claimed at
/// build time. Re-signing a draft selects from this set (when non-empty) so
/// the signed inputs are always the reserved ones: selecting from the full
/// pool instead could drift onto a larger coin that synced in after the
/// build, which was never reserved and could be claimed by another draft
/// before this one broadcasts. Selection over exactly the build-time set is
/// deterministic (same coins, same largest-first order), so the re-signed tx
/// spends what the preview promised.
pub fn load_reserved_coins(
    conn: &Connection,
    profile_id: &str,
    draft_id: &str,
) -> Result<Vec<SpendableCoin>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT u.txid, u.vout, u.value_doos, d.branch, d.child_index
         FROM tracked_utxos u
         JOIN derived_addresses d
           ON d.wallet_profile_id = u.wallet_profile_id
          AND d.address = u.address
         WHERE u.wallet_profile_id = ?1
           AND u.reserved_by_draft_id = ?2
           AND u.spent_by_txid IS NULL
           AND u.covenant_type = 0
           AND u.spend_class = 'liquid_hns'
         ORDER BY u.value_doos DESC, u.txid ASC, u.vout ASC",
    )?;
    let rows = stmt.query_map(params![profile_id, draft_id], |row| {
        Ok(SpendableCoin {
            txid: row.get(0)?,
            vout: row.get::<_, i64>(1)? as u32,
            value: row.get::<_, i64>(2)? as u64,
            branch: row.get::<_, i64>(3)? as u32,
            child_index: row.get::<_, i64>(4)? as u32,
        })
    })?;
    let mut coins = Vec::new();
    for c in rows {
        coins.push(c?);
    }
    Ok(coins)
}

/// Load all spendable coins for a profile: unspent, covenant-free
/// (`covenant_type = 0`), and classified as liquid HNS.
///
/// Coins live in `tracked_utxos`, but the BIP44 `(branch, child_index)` needed
/// to re-derive each input's signing key lives in `derived_addresses`. We join
/// the two on `(wallet_profile_id, address)` so each [`SpendableCoin`] carries
/// the derivation path. A coin is unspent when `spent_by_txid IS NULL`.
/// Ordered largest-first for deterministic selection.
///
/// Coins reserved by another, still-live draft (I3) are excluded — two drafts
/// built before either broadcasts must not be able to select the same UTXOs.
/// `own_draft_id`, when set, is the id of the draft this call is re-selecting
/// for (e.g. re-signing an existing draft): coins that draft itself already
/// reserved remain visible to it. Stale reservations are released first (see
/// [`release_stale_reservations`]) so they never wrongly exclude a coin.
pub fn load_spendable_coins(
    conn: &Connection,
    profile_id: &str,
    own_draft_id: Option<&str>,
) -> Result<Vec<SpendableCoin>, AppError> {
    release_stale_reservations(conn, profile_id)?;

    let mut stmt = conn.prepare(
        "SELECT u.txid, u.vout, u.value_doos, d.branch, d.child_index
         FROM tracked_utxos u
         JOIN derived_addresses d
           ON d.wallet_profile_id = u.wallet_profile_id
          AND d.address = u.address
         WHERE u.wallet_profile_id = ?1
           AND u.spent_by_txid IS NULL
           AND u.covenant_type = 0
           AND u.spend_class = 'liquid_hns'
           AND (u.reserved_by_draft_id IS NULL OR u.reserved_by_draft_id = ?2)
         ORDER BY u.value_doos DESC, u.txid ASC, u.vout ASC",
    )?;
    let rows = stmt.query_map(params![profile_id, own_draft_id], |row| {
        Ok(SpendableCoin {
            txid: row.get(0)?,
            vout: row.get::<_, i64>(1)? as u32,
            value: row.get::<_, i64>(2)? as u64,
            branch: row.get::<_, i64>(3)? as u32,
            child_index: row.get::<_, i64>(4)? as u32,
        })
    })?;
    let mut coins = Vec::new();
    for c in rows {
        coins.push(c?);
    }
    Ok(coins)
}

/// The outcome of coin selection: the coins to spend, the fee, and the change
/// amount (0 if no change output should be created).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub coins: Vec<SpendableCoin>,
    pub fee: u64,
    pub change: u64,
    /// Total value of the selected inputs.
    pub input_total: u64,
}

/// Select coins (largest-first) to cover `amount` plus the fee, accounting for
/// a change output when one is warranted.
///
/// The fee is recomputed as inputs are added (more inputs = larger tx = higher
/// fee). A change output is included only if the leftover after `amount + fee`
/// (with the change output's own bytes priced in) exceeds [`DUST_THRESHOLD`];
/// otherwise the leftover is absorbed into the fee and no change is created.
pub fn select_coins(
    available: &[SpendableCoin],
    amount: u64,
    rate_per_byte: u64,
) -> Result<Selection, AppError> {
    if amount == 0 {
        return Err(AppError::InvalidInput(
            "send amount must be greater than zero".to_string(),
        ));
    }
    if amount < DUST_THRESHOLD {
        return Err(AppError::InvalidInput(format!(
            "send amount {amount} is below the dust threshold {DUST_THRESHOLD}"
        )));
    }

    let mut selected: Vec<SpendableCoin> = Vec::new();
    let mut input_total: u64 = 0;

    for coin in available {
        selected.push(coin.clone());
        input_total = input_total.saturating_add(coin.value);

        let n_inputs = selected.len() as u64;

        // Fee assuming a change output exists (recipient + change = 2 outputs).
        let fee_with_change = estimate_fee(n_inputs, 2, rate_per_byte);
        // Fee assuming no change (recipient only = 1 output).
        let fee_no_change = estimate_fee(n_inputs, 1, rate_per_byte);

        // Can we cover amount + fee while producing change above dust?
        if input_total >= amount.saturating_add(fee_with_change) {
            let change = input_total - amount - fee_with_change;
            if change >= DUST_THRESHOLD {
                return Ok(Selection {
                    coins: selected,
                    fee: fee_with_change,
                    change,
                    input_total,
                });
            }
            // Change would be dust: fold it into the fee, drop the change output.
            return Ok(Selection {
                coins: selected,
                fee: input_total - amount,
                change: 0,
                input_total,
            });
        }

        // Otherwise, can we cover amount + fee with NO change output exactly
        // (or with a dust remainder that becomes extra fee)?
        if input_total >= amount.saturating_add(fee_no_change) {
            return Ok(Selection {
                coins: selected,
                fee: input_total - amount,
                change: 0,
                input_total,
            });
        }
        // Not enough yet — add another coin.
    }

    Err(AppError::InvalidInput(
        "insufficient funds to cover amount and fee".to_string(),
    ))
}

/// Sweep selection: spend ALL available coins into a single recipient output of
/// `input_total - fee` (no change). Used by "Send Max". The recipient amount is
/// `input_total - fee`; the caller reads it as `input_total - selection.fee`.
pub fn select_all_coins(
    available: &[SpendableCoin],
    rate_per_byte: u64,
) -> Result<Selection, AppError> {
    if available.is_empty() {
        return Err(AppError::InvalidInput(
            "no spendable coins to send".to_string(),
        ));
    }
    let input_total: u64 = available.iter().map(|c| c.value).sum();
    // One recipient output, no change.
    let fee = estimate_fee(available.len() as u64, 1, rate_per_byte);
    if input_total <= fee || input_total - fee < DUST_THRESHOLD {
        return Err(AppError::InvalidInput(format!(
            "balance ({input_total}) is too low to cover the network fee ({fee})"
        )));
    }
    Ok(Selection {
        coins: available.to_vec(),
        fee,
        change: 0,
        input_total,
    })
}

/// Convert an hsrd txid hex into the 32-byte prevout hash used by [`Outpoint`].
///
/// Handshake does NOT byte-reverse hashes (unlike Bitcoin): the hash string the
/// node reports for a coin is the exact byte order written into a spending
/// input's prevout. So this is a plain hex decode with NO reversal — reversing
/// would reference a non-existent outpoint and the node would reject the spend.
fn outpoint_hash_from_txid(txid: &str) -> Result<[u8; 32], AppError> {
    let bytes =
        hex::decode(txid).map_err(|e| AppError::InvalidInput(format!("bad txid hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(AppError::InvalidInput(format!(
            "txid must be 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(hash)
}

/// The result of building a send: the signed tx hex plus a summary the UI can
/// display for confirmation before broadcast.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BuiltTransaction {
    /// Fully-signed transaction, hex-encoded for `sendrawtransaction`.
    pub tx_hex: String,
    /// Transaction id in hsrd natural-order hex (no Bitcoin-style reversal).
    pub txid: String,
    pub fee: u64,
    pub input_total: u64,
    /// Sum of recipient + change outputs.
    pub output_total: u64,
    pub change: u64,
    pub num_inputs: usize,
}

/// Build and sign a standard (plain) HNS send.
///
/// * `session` — unlocked signer providing the BIP32 master key.
/// * `network` — selects address HRP and BIP44 coin type.
/// * `available` — spendable coins (see [`load_spendable_coins`]).
/// * `to_address` — destination bech32 address (validated up front).
/// * `amount` — amount to send in dollarydoos.
/// * `change_address` — wallet-owned address for change.
/// * `rate_per_byte` — fee rate in dollarydoos per byte.
///
/// Returns the fully-signed transaction plus a summary.
#[allow(clippy::too_many_arguments)]
pub fn build_send(
    session: &mut SignerSession,
    network: Network,
    account: u32,
    available: &[SpendableCoin],
    to_address: &str,
    amount: u64,
    change_address: &str,
    rate_per_byte: u64,
    max: bool,
) -> Result<BuiltTransaction, AppError> {
    // Validate both addresses before touching keys so we fail fast.
    let to_output_addr = output_address_from_string(network, to_address)?;
    let change_output_addr = output_address_from_string(network, change_address)?;

    // Send Max sweeps all coins into one output of `input_total - fee` (no
    // change); otherwise select coins to cover `amount` + fee.
    let selection = if max {
        select_all_coins(available, rate_per_byte)?
    } else {
        select_coins(available, amount, rate_per_byte)?
    };
    let recipient_amount = if max {
        selection.input_total - selection.fee
    } else {
        amount
    };

    // Recipient output first, then change (only if above dust — select_coins
    // already folded dust change into the fee, so `change == 0` means none).
    let mut outputs = vec![Output {
        value: recipient_amount,
        address: to_output_addr,
        covenant: Covenant::default(),
    }];
    if selection.change > 0 {
        outputs.push(Output {
            value: selection.change,
            address: change_output_addr,
            covenant: Covenant::default(),
        });
    }
    let output_total = outputs.iter().map(|o| o.value).sum();

    // Build unsigned inputs from the selected coins.
    let mut tx = Transaction::new();
    for coin in &selection.coins {
        let hash = outpoint_hash_from_txid(&coin.txid)?;
        tx.inputs.push(Input::new(Outpoint {
            hash,
            index: coin.vout,
        }));
    }
    tx.outputs = outputs;

    // Re-derive each input's signing key and sign as P2WPKH (SIGHASH_ALL).
    let master = session.master()?;
    for (i, coin) in selection.coins.iter().enumerate() {
        let path =
            crate::noncustodial::hd::bip44_path(network, account, coin.branch, coin.child_index);
        let child = master.derive_path(&path)?;
        let pubkey = child.compressed_pubkey();
        let hash160 = address::pubkey_to_hash160(&pubkey);
        tx.sign_p2wpkh_input(i, &child.secret, &hash160, coin.value, sighash::ALL)?;
    }

    let tx_hex = tx.to_hex();
    let txid = tx.txid();

    Ok(BuiltTransaction {
        tx_hex,
        txid,
        fee: selection.fee,
        input_total: selection.input_total,
        output_total,
        change: selection.change,
        num_inputs: selection.coins.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::hd::ExtendedPrivKey;
    use rusqlite::Connection;

    fn test_session() -> SignerSession {
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).expect("master");
        SignerSession::unlock("p1".to_string(), Network::Main, master, 60_000)
    }

    const ACCOUNT: u32 = 0;

    fn coin(txid_byte: u8, value: u64, branch: u32, child: u32) -> SpendableCoin {
        SpendableCoin {
            txid: hex::encode([txid_byte; 32]),
            vout: 0,
            value,
            branch,
            child_index: child,
        }
    }

    #[test]
    fn estimate_size_and_fee_are_monotonic() {
        assert!(estimate_size(2, 2) > estimate_size(1, 1));
        assert!(estimate_fee(2, 2, 5) > estimate_fee(1, 1, 5));
        // rate below the relay floor is clamped up to the floor.
        assert_eq!(
            estimate_fee(1, 1, 0),
            estimate_size(1, 1) * MIN_FEE_RATE_PER_BYTE
        );
    }

    #[test]
    fn select_coins_rejects_zero_and_dust_amounts() {
        let coins = vec![coin(1, 100_000, 0, 0)];
        assert!(matches!(
            select_coins(&coins, 0, 1).unwrap_err(),
            AppError::InvalidInput(_)
        ));
        assert!(matches!(
            select_coins(&coins, DUST_THRESHOLD - 1, 1).unwrap_err(),
            AppError::InvalidInput(_)
        ));
    }

    #[test]
    fn select_coins_creates_change_when_above_dust() {
        let coins = vec![coin(1, 1_000_000, 0, 0)];
        let sel = select_coins(&coins, 100_000, 1).expect("selection");
        assert_eq!(sel.coins.len(), 1);
        assert!(sel.change >= DUST_THRESHOLD);
        // Conservation: inputs == amount + fee + change.
        assert_eq!(sel.input_total, 100_000 + sel.fee + sel.change);
    }

    #[test]
    fn select_coins_folds_dust_change_into_fee() {
        // Pick an input that exactly covers amount + a tiny remainder so the
        // leftover would be dust and gets folded into the fee.
        let amount = 100_000u64;
        let fee_with_change = estimate_fee(1, 2, 1);
        // input = amount + fee_with_change + (dust-1) so change < dust.
        let input = amount + fee_with_change + (DUST_THRESHOLD - 1);
        let coins = vec![coin(1, input, 0, 0)];
        let sel = select_coins(&coins, amount, 1).expect("selection");
        assert_eq!(sel.change, 0);
        // All non-amount value became fee.
        assert_eq!(sel.fee, input - amount);
        assert_eq!(sel.input_total, amount + sel.fee);
    }

    #[test]
    fn select_coins_insufficient_funds_errors() {
        let coins = vec![coin(1, 1000, 0, 0)];
        let err = select_coins(&coins, 500_000, 1).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    /// In-memory DB with the FULL migration chain (needed for
    /// `wallet_tx_drafts` + `reserved_by_draft_id`, used by the reservation
    /// tests below) and a single profile row, mirroring `derivation.rs`'s
    /// test fixture.
    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        conn.execute(
            "INSERT INTO wallet_profiles (id, label, kind, network, account_xpub)
             VALUES ('p1', 'Test', 'watch_only_xpub', 'mainnet', 'xpubPLACEHOLDER')",
            [],
        )
        .unwrap();
        conn
    }

    fn insert_derived(conn: &Connection, branch: u32, child: u32, address: &str) {
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES ('p1', 0, ?1, ?2, ?3, '0014deadbeef', '02deadbeef')",
            params![branch as i64, child as i64, address],
        )
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_utxo(
        conn: &Connection,
        txid: &str,
        vout: u32,
        address: &str,
        value: u64,
        covenant_type: u8,
        spend_class: &str,
        spent_by_txid: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex,
                 value_doos, covenant_type, spend_class, spent_by_txid)
             VALUES (?1, ?2, 'p1', ?3, '0014deadbeef', ?4, ?5, ?6, ?7)",
            params![
                txid,
                vout as i64,
                address,
                value as i64,
                covenant_type as i64,
                spend_class,
                spent_by_txid,
            ],
        )
        .unwrap();
    }

    #[test]
    fn load_spendable_coins_joins_addresses_and_filters() {
        let conn = mem_db();
        let txid_a = hex::encode([0xaa; 32]);
        let txid_b = hex::encode([0xbb; 32]);
        let txid_c = hex::encode([0xcc; 32]);
        let txid_d = hex::encode([0xdd; 32]);
        let txid_e = hex::encode([0xee; 32]);

        insert_derived(&conn, 0, 5, "hs1qrecv");
        insert_derived(&conn, 1, 9, "hs1qchange");

        // Spendable: liquid, unspent, covenant-free, address is ours.
        insert_utxo(
            &conn,
            &txid_a,
            0,
            "hs1qrecv",
            300_000,
            0,
            "liquid_hns",
            None,
        );
        insert_utxo(
            &conn,
            &txid_b,
            1,
            "hs1qchange",
            700_000,
            0,
            "liquid_hns",
            None,
        );
        // Excluded: already spent.
        insert_utxo(
            &conn,
            &txid_c,
            0,
            "hs1qrecv",
            999_999,
            0,
            "liquid_hns",
            Some("somespender"),
        );
        // Excluded: carries a name covenant.
        insert_utxo(
            &conn,
            &txid_d,
            0,
            "hs1qrecv",
            999_999,
            7,
            "name_control",
            None,
        );
        // Excluded: address not in derived_addresses (no join row).
        insert_utxo(
            &conn,
            &txid_e,
            0,
            "hs1qforeign",
            999_999,
            0,
            "liquid_hns",
            None,
        );

        let coins = load_spendable_coins(&conn, "p1", None).expect("load");

        // Only the two genuinely-spendable coins, largest-first.
        assert_eq!(coins.len(), 2);
        assert_eq!(coins[0].value, 700_000);
        assert_eq!(coins[0].txid, txid_b);
        assert_eq!(coins[0].branch, 1);
        assert_eq!(coins[0].child_index, 9);
        assert_eq!(coins[1].value, 300_000);
        assert_eq!(coins[1].txid, txid_a);
        assert_eq!(coins[1].branch, 0);
        assert_eq!(coins[1].child_index, 5);
    }

    /// Insert a minimal `wallet_tx_drafts` row for reservation tests, with a
    /// controllable `created_at` so TTL expiry can be exercised.
    fn insert_draft_row(conn: &Connection, id: &str, created_at: Option<&str>) {
        conn.execute(
            "INSERT INTO wallet_tx_drafts
                (id, wallet_profile_id, action, unsigned_tx_hex, signing_inputs_json, summary_json)
             VALUES (?1, 'p1', 'send_hns', '', '{}', '{}')",
            params![id],
        )
        .unwrap();
        if let Some(ts) = created_at {
            conn.execute(
                "UPDATE wallet_tx_drafts SET created_at = ?1 WHERE id = ?2",
                params![ts, id],
            )
            .unwrap();
        }
    }

    fn reserve(conn: &Connection, txid: &str, vout: u32, draft_id: &str) {
        conn.execute(
            "UPDATE tracked_utxos SET reserved_by_draft_id = ?1
             WHERE wallet_profile_id = 'p1' AND txid = ?2 AND vout = ?3",
            params![draft_id, txid, vout as i64],
        )
        .unwrap();
    }

    #[test]
    fn load_spendable_coins_excludes_coins_reserved_by_another_live_draft() {
        let conn = mem_db();
        let txid_a = hex::encode([0xaa; 32]);
        insert_derived(&conn, 0, 5, "hs1qrecv");
        insert_utxo(
            &conn,
            &txid_a,
            0,
            "hs1qrecv",
            300_000,
            0,
            "liquid_hns",
            None,
        );
        insert_draft_row(&conn, "draft-a", None);
        reserve(&conn, &txid_a, 0, "draft-a");

        // A fresh selection (no own draft id) must not see draft-a's coin.
        let coins = load_spendable_coins(&conn, "p1", None).expect("load");
        assert!(coins.is_empty(), "reserved coin must be excluded");

        // A DIFFERENT draft's re-selection must also not see it.
        let coins = load_spendable_coins(&conn, "p1", Some("draft-b")).expect("load");
        assert!(
            coins.is_empty(),
            "coin reserved by another draft stays excluded"
        );
    }

    #[test]
    fn load_spendable_coins_includes_coins_reserved_by_own_draft_id() {
        let conn = mem_db();
        let txid_a = hex::encode([0xaa; 32]);
        insert_derived(&conn, 0, 5, "hs1qrecv");
        insert_utxo(
            &conn,
            &txid_a,
            0,
            "hs1qrecv",
            300_000,
            0,
            "liquid_hns",
            None,
        );
        insert_draft_row(&conn, "draft-a", None);
        reserve(&conn, &txid_a, 0, "draft-a");

        // Re-selecting for the SAME draft (e.g. re-signing) must see its own
        // reserved coin.
        let coins = load_spendable_coins(&conn, "p1", Some("draft-a")).expect("load");
        assert_eq!(coins.len(), 1);
        assert_eq!(coins[0].txid, txid_a);
    }

    #[test]
    fn load_spendable_coins_reclaims_ttl_expired_reservation() {
        let conn = mem_db();
        let txid_a = hex::encode([0xaa; 32]);
        insert_derived(&conn, 0, 5, "hs1qrecv");
        insert_utxo(
            &conn,
            &txid_a,
            0,
            "hs1qrecv",
            300_000,
            0,
            "liquid_hns",
            None,
        );
        // A draft created well past the TTL — its claim is stale.
        insert_draft_row(&conn, "draft-old", Some("2000-01-01T00:00:00Z"));
        reserve(&conn, &txid_a, 0, "draft-old");

        let coins = load_spendable_coins(&conn, "p1", None).expect("load");
        assert_eq!(coins.len(), 1, "TTL-expired reservation must be reclaimed");
        assert_eq!(coins[0].txid, txid_a);

        // The stale reservation was also opportunistically cleared in the DB,
        // not just skipped for this one read.
        let reserved: Option<String> = conn
            .query_row(
                "SELECT reserved_by_draft_id FROM tracked_utxos WHERE txid = ?1",
                params![txid_a],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            reserved.is_none(),
            "expired reservation should be cleared, not just ignored"
        );
    }

    /// Finding 1 (Task 5 review): a reservation belonging to a draft that has
    /// already reached the chain (or may have — `broadcast_pending`, Finding
    /// 2) must NOT be swept by the TTL just because `created_at` (build time,
    /// never refreshed on broadcast) is old. Freeing it would reopen a
    /// double-select window while `tracked_utxos.spent_by_txid` is still
    /// NULL pending the next sync.
    #[test]
    fn load_spendable_coins_keeps_ttl_expired_reservation_of_an_in_flight_draft() {
        for status in ["broadcasted", "confirmed", "broadcast_pending"] {
            let conn = mem_db();
            let txid_a = hex::encode([0xaa; 32]);
            insert_derived(&conn, 0, 5, "hs1qrecv");
            insert_utxo(
                &conn,
                &txid_a,
                0,
                "hs1qrecv",
                300_000,
                0,
                "liquid_hns",
                None,
            );
            // Well past RESERVATION_TTL_SECS.
            insert_draft_row(&conn, "draft-inflight", Some("2000-01-01T00:00:00Z"));
            conn.execute(
                "UPDATE wallet_tx_drafts SET status = ?1 WHERE id = 'draft-inflight'",
                params![status],
            )
            .unwrap();
            reserve(&conn, &txid_a, 0, "draft-inflight");

            let coins = load_spendable_coins(&conn, "p1", None).expect("load");
            assert!(
                coins.is_empty(),
                "status={status}: an in-flight draft's coin must stay reserved past the TTL"
            );

            let reserved: Option<String> = conn
                .query_row(
                    "SELECT reserved_by_draft_id FROM tracked_utxos WHERE txid = ?1",
                    params![txid_a],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                reserved.as_deref(),
                Some("draft-inflight"),
                "status={status}: reservation must not be cleared by the TTL sweep"
            );
        }
    }

    /// Control for Finding 1: a draft that was only ever built (never signed
    /// or broadcast, `status = 'draft'`) must still be released once its
    /// reservation crosses the TTL — the status guard must not accidentally
    /// make ALL expired reservations sticky.
    #[test]
    fn load_spendable_coins_still_reclaims_ttl_expired_reservation_of_a_never_signed_draft() {
        let conn = mem_db();
        let txid_a = hex::encode([0xaa; 32]);
        insert_derived(&conn, 0, 5, "hs1qrecv");
        insert_utxo(
            &conn,
            &txid_a,
            0,
            "hs1qrecv",
            300_000,
            0,
            "liquid_hns",
            None,
        );
        // insert_draft_row leaves status at its table default, 'draft'.
        insert_draft_row(&conn, "draft-abandoned", Some("2000-01-01T00:00:00Z"));
        reserve(&conn, &txid_a, 0, "draft-abandoned");

        let coins = load_spendable_coins(&conn, "p1", None).expect("load");
        assert_eq!(
            coins.len(),
            1,
            "a built-but-never-signed draft's TTL-expired reservation must still be reclaimed"
        );
        assert_eq!(coins[0].txid, txid_a);
    }

    #[test]
    fn load_spendable_coins_reclaims_dangling_reservation() {
        // A reservation pointing at a draft id that no longer exists (e.g. the
        // draft row was deleted without going through the release path) must
        // not permanently lock the coin out of selection.
        let conn = mem_db();
        let txid_a = hex::encode([0xaa; 32]);
        insert_derived(&conn, 0, 5, "hs1qrecv");
        insert_utxo(
            &conn,
            &txid_a,
            0,
            "hs1qrecv",
            300_000,
            0,
            "liquid_hns",
            None,
        );
        reserve(&conn, &txid_a, 0, "no-such-draft");

        let coins = load_spendable_coins(&conn, "p1", None).expect("load");
        assert_eq!(coins.len(), 1, "dangling reservation must be reclaimed");
    }

    #[test]
    fn load_spendable_coins_empty_when_none() {
        let conn = mem_db();
        assert!(load_spendable_coins(&conn, "p1", None).unwrap().is_empty());
    }

    #[test]
    fn outpoint_hash_preserves_byte_order() {
        // Handshake does NOT byte-reverse hashes: the prevout hash must be the
        // node's coin-hash bytes verbatim, else the spend references a
        // non-existent outpoint and is rejected.
        let mut h = [0u8; 32];
        for (i, b) in h.iter_mut().enumerate() {
            *b = i as u8;
        }
        let txid = hex::encode(h);
        let decoded = outpoint_hash_from_txid(&txid).expect("hash");
        assert_eq!(decoded, h, "prevout hash must match the txid bytes exactly");
    }

    #[test]
    fn outpoint_hash_rejects_wrong_length() {
        assert!(outpoint_hash_from_txid("00").is_err());
        assert!(outpoint_hash_from_txid("zz").is_err());
    }

    #[test]
    fn build_send_produces_signed_tx() {
        let mut session = test_session();
        let coins = vec![coin(1, 1_000_000, 0, 0), coin(2, 500_000, 0, 1)];
        let built = build_send(
            &mut session,
            Network::Main,
            ACCOUNT,
            &coins,
            "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx",
            120_000,
            "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx",
            1,
            false,
        )
        .expect("build");
        assert!(built.num_inputs >= 1);
        assert!(!built.tx_hex.is_empty());
        assert_eq!(built.txid.len(), 64);
        assert!(built.fee > 0);
        // Conservation across the whole tx.
        assert_eq!(built.input_total, built.output_total + built.fee);
    }

    #[test]
    fn build_send_conserves_value_and_fee_equals_rate_times_size() {
        let mut session = test_session();
        let rate = 3;
        let addr = "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx";
        // Single input, comfortably above amount + fee => one change output.
        let coins = vec![coin(1, 2_000_000, 0, 0)];
        let built = build_send(
            &mut session,
            Network::Main,
            ACCOUNT,
            &coins,
            addr,
            500_000,
            addr,
            rate,
            false,
        )
        .expect("build");
        // Exact conservation: inputs == outputs + fee.
        assert_eq!(built.input_total, built.output_total + built.fee);
        // With change present the tx is 1-in/2-out; fee == size * rate exactly.
        assert!(built.change > 0);
        assert_eq!(built.fee, estimate_fee(1, 2, rate));
        assert_eq!(built.fee, estimate_size(1, 2) * rate);
    }

    #[test]
    fn build_send_rejects_bad_destination_address() {
        let mut session = test_session();
        let coins = vec![coin(1, 1_000_000, 0, 0)];
        let err = build_send(
            &mut session,
            Network::Main,
            ACCOUNT,
            &coins,
            "not-an-address",
            120_000,
            "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx",
            1,
            false,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            AppError::Crypto(_) | AppError::InvalidInput(_)
        ));
    }

    #[test]
    fn select_all_coins_sweeps_all_with_no_change() {
        let coins = vec![coin(1, 1_000_000, 0, 0), coin(2, 2_000_000, 0, 1)];
        let rate = 1;
        let sel = select_all_coins(&coins, rate).expect("sweep");
        assert_eq!(sel.coins.len(), 2, "spends every coin");
        assert_eq!(sel.change, 0, "sweep has no change");
        assert_eq!(sel.input_total, 3_000_000);
        // One recipient output, no change.
        assert_eq!(sel.fee, estimate_fee(2, 1, rate));
        // Recipient receives input_total - fee.
        assert_eq!(
            sel.input_total - sel.fee,
            3_000_000 - estimate_fee(2, 1, rate)
        );
    }

    #[test]
    fn select_all_coins_rejects_when_balance_below_fee_plus_dust() {
        let coins = vec![coin(1, 500, 0, 0)]; // can't cover fee + dust
        let err = select_all_coins(&coins, 1).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
    }
}
