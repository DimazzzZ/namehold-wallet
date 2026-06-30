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
//! Fee policy is verified against hsd `lib/protocol/policy.js`:
//!   - `MIN_RELAY = 1000` dollarydoos per 1000 bytes (1 dood/byte floor).
//!   - Dust is computed from the output size at the min relay rate; for a
//!     standard 31-byte P2WPKH output hsd's threshold works out well under
//!     `DUST_THRESHOLD`. We use a conservative fixed dust floor below.

use rusqlite::{params, Connection};

use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::network::Network;
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::tx::{
    output_address_from_string, sighash, Covenant, Input, Outpoint, Output, Transaction,
};

/// Minimum relay fee rate in dollarydoos per byte (hsd `MIN_RELAY` is 1000
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
    /// Funding transaction id in hsd natural-order hex (as stored / as the node
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

/// Load all spendable coins for a profile: unspent, covenant-free
/// (`covenant_type = 0`), and classified as liquid HNS.
///
/// Coins live in `tracked_utxos`, but the BIP44 `(branch, child_index)` needed
/// to re-derive each input's signing key lives in `derived_addresses`. We join
/// the two on `(wallet_profile_id, address)` so each [`SpendableCoin`] carries
/// the derivation path. A coin is unspent when `spent_by_txid IS NULL`.
/// Ordered largest-first for deterministic selection.
pub fn load_spendable_coins(
    conn: &Connection,
    profile_id: &str,
) -> Result<Vec<SpendableCoin>, AppError> {
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
         ORDER BY u.value_doos DESC, u.txid ASC, u.vout ASC",
    )?;
    let rows = stmt.query_map(params![profile_id], |row| {
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

/// Convert an hsd txid hex into the 32-byte prevout hash used by [`Outpoint`].
///
/// Handshake does NOT byte-reverse hashes (unlike Bitcoin): the hash string the
/// node reports for a coin is the exact byte order written into a spending
/// input's prevout. So this is a plain hex decode with NO reversal — reversing
/// would reference a non-existent outpoint and the node would reject the spend.
fn outpoint_hash_from_txid(txid: &str) -> Result<[u8; 32], AppError> {
    let bytes = hex::decode(txid)
        .map_err(|e| AppError::InvalidInput(format!("bad txid hex: {e}")))?;
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
    /// Transaction id in hsd natural-order hex (no Bitcoin-style reversal).
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
        let path = crate::noncustodial::hd::bip44_path(
            network,
            account,
            coin.branch,
            coin.child_index,
        );
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

    /// In-memory DB with the non-custodial profile + chain-cache schema and a
    /// single profile row, mirroring `derivation.rs`'s test fixture.
    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../sql/006_noncustodial_wallet_profiles.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/007_noncustodial_chain_cache.sql"))
            .unwrap();
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
        insert_utxo(&conn, &txid_a, 0, "hs1qrecv", 300_000, 0, "liquid_hns", None);
        insert_utxo(&conn, &txid_b, 1, "hs1qchange", 700_000, 0, "liquid_hns", None);
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
        insert_utxo(&conn, &txid_d, 0, "hs1qrecv", 999_999, 7, "name_control", None);
        // Excluded: address not in derived_addresses (no join row).
        insert_utxo(&conn, &txid_e, 0, "hs1qforeign", 999_999, 0, "liquid_hns", None);

        let coins = load_spendable_coins(&conn, "p1").expect("load");

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

    #[test]
    fn load_spendable_coins_empty_when_none() {
        let conn = mem_db();
        assert!(load_spendable_coins(&conn, "p1").unwrap().is_empty());
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
        let built =
            build_send(&mut session, Network::Main, ACCOUNT, &coins, addr, 500_000, addr, rate, false)
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
        assert_eq!(sel.input_total - sel.fee, 3_000_000 - estimate_fee(2, 1, rate));
    }

    #[test]
    fn select_all_coins_rejects_when_balance_below_fee_plus_dust() {
        let coins = vec![coin(1, 500, 0, 0)]; // can't cover fee + dust
        let err = select_all_coins(&coins, 1).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "got {err:?}");
    }
}
