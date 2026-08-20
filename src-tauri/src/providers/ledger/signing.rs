//! High-level Ledger transaction signing orchestrator.
//!
//! Drives the parse → sign flow end-to-end for a given [`DraftPlan`], returning
//! the signed transaction hex and txid — the same contract as the hot-wallet
//! [`sign_plan`](crate::noncustodial::actions::sign_plan).
//!
//! The caller provides the account xpub (stored in the profile) so per-input
//! pubkeys can be derived locally (no extra device round-trips). The device is
//! driven through:
//!   1. **Parse mode** — the full transaction blob (inputs, outputs, covenants).
//!   2. **Sign mode** — one APDU sequence per input; the device returns a 65-byte
//!      compact signature.
//!
//! The resulting signatures are assembled into the transaction's witness stack
//! and the final signed hex is produced.

use crate::error::AppError;
use crate::noncustodial::actions::DraftPlan;
use crate::noncustodial::hd::ExtendedPubKey;
use crate::noncustodial::network::Network;

use super::apdu::network_flag;
use super::hid_transport::HidIo;
use super::parse_mode::{build_parse_apdus, ChangeInfo, OutputName};
use super::sign_mode::{build_sign_apdus, parse_signature, SignInput};
use super::LedgerSigner;

/// Sign a [`DraftPlan`] using the connected Ledger device.
///
/// * `device` — live connection to the Handshake app.
/// * `plan` — the persisted draft plan (inputs, outputs, covenants).
/// * `account_xpub` — the account-level extended public key (stored in profile).
/// * `network` — the active network.
/// * `change` — optional change output metadata (so the device skips prompting).
/// * `names` — human-readable names for name-bearing covenant outputs.
///
/// Returns `(signed_tx_hex, txid)`.
pub fn sign_transaction<T: HidIo>(
    device: &mut LedgerSigner<T>,
    plan: &DraftPlan,
    account_xpub: &ExtendedPubKey,
    network: Network,
    change: Option<&ChangeInfo>,
    names: &[OutputName],
) -> Result<(String, String), AppError> {
    let net_flag = network_flag(network);

    // --- Phase 1: Parse mode (stream the full tx to the device) ---
    let parse_cmds = build_parse_apdus(plan, network, change, names, net_flag)?;
    for cmd in &parse_cmds {
        // All parse-mode responses are empty (just SW 0x9000); the device
        // accumulates state internally.
        device.transport_mut().exchange_ok(cmd)?;
    }

    // --- Phase 2: Sign mode (one request per input) ---
    let mut signatures: Vec<[u8; 65]> = Vec::with_capacity(plan.inputs.len());
    for inp in &plan.inputs {
        // Derive the pubkey locally from the account xpub.
        let child = account_xpub.derive_path(&[inp.branch, inp.child_index])?;
        let pubkey = child.compressed_pubkey();

        let prevout_hash = hex_to_32(&inp.txid)?;

        let sign_cmds = build_sign_apdus(&SignInput {
            network,
            account: plan.account,
            branch: inp.branch,
            child_index: inp.child_index,
            sighash_type: inp.sighash_type,
            prevout_hash,
            prevout_index: inp.vout,
            value: inp.value,
            sequence: 0xFFFF_FFFF, // always final
            pubkey,
        })?;

        // Send all but the last APDU (intermediate ones return empty).
        let last_idx = sign_cmds.len() - 1;
        for cmd in &sign_cmds[..last_idx] {
            device.transport_mut().exchange_ok(cmd)?;
        }
        // The last APDU returns the 65-byte signature.
        let sig_body = device.transport_mut().exchange_ok(&sign_cmds[last_idx])?;
        signatures.push(parse_signature(&sig_body)?);
    }

    // --- Phase 3: Assemble the signed transaction ---
    let mut tx = crate::noncustodial::actions::rebuild_unsigned(plan, network)?;
    for (i, inp) in plan.inputs.iter().enumerate() {
        let child = account_xpub.derive_path(&[inp.branch, inp.child_index])?;
        let pubkey = child.compressed_pubkey();
        // Witness: [signature, pubkey]
        tx.inputs[i].witness = vec![signatures[i].to_vec(), pubkey.to_vec()];
    }

    Ok((tx.to_hex(), tx.txid()))
}

fn hex_to_32(hex_str: &str) -> Result<[u8; 32], AppError> {
    let bytes =
        hex::decode(hex_str).map_err(|e| AppError::InvalidInput(format!("bad txid hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(AppError::InvalidInput(format!(
            "txid must be 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::hd::bip44_path;
    use crate::noncustodial::hd::ExtendedPrivKey;
    use crate::noncustodial::network::Network;
    use crate::noncustodial::sync::COV_NONE;
    use crate::providers::ledger::hid_transport::{Transport, PACKET_SIZE};
    use std::collections::VecDeque;

    /// A mock that auto-responds to parse-mode APDUs with 0x9000 and to the
    /// final sign-mode APDU with a dummy 65-byte signature.
    struct AutoSignHid {
        /// How many write packets we've seen.
        write_count: usize,
        /// Pre-loaded response packets (one per exchange).
        responses: VecDeque<Vec<[u8; PACKET_SIZE]>>,
    }

    impl AutoSignHid {
        fn new(num_parse_exchanges: usize, num_sign_exchanges: usize) -> Self {
            let mut responses = VecDeque::new();
            // Parse-mode: each exchange returns just SW 0x9000 (empty body).
            for _ in 0..num_parse_exchanges {
                responses.push_back(frame_response(&[], 0x9000));
            }
            // Sign-mode: intermediate exchanges return empty, last returns 65 bytes.
            for i in 0..num_sign_exchanges {
                if i == num_sign_exchanges - 1 {
                    // Dummy signature: 32-byte r, 32-byte s, 1-byte sighash type.
                    let mut sig = vec![0x30u8; 64];
                    sig.push(0x01); // sighash ALL
                    responses.push_back(frame_response(&sig, 0x9000));
                } else {
                    responses.push_back(frame_response(&[], 0x9000));
                }
            }
            Self {
                write_count: 0,
                responses,
            }
        }

        /// Create a mock for multi-input transactions where each input's sign
        /// blob fits in 1 APDU (common case). Every sign exchange returns a
        /// signature.
        fn multi_input(num_parse_exchanges: usize, num_inputs: usize) -> Self {
            let mut responses = VecDeque::new();
            for _ in 0..num_parse_exchanges {
                responses.push_back(frame_response(&[], 0x9000));
            }
            // Each input produces exactly 1 sign exchange that returns a signature.
            for _ in 0..num_inputs {
                let mut sig = vec![0x30u8; 64];
                sig.push(0x01);
                responses.push_back(frame_response(&sig, 0x9000));
            }
            Self {
                write_count: 0,
                responses,
            }
        }

        /// Create a mock that rejects on a specific exchange index (0-based).
        fn rejecting_at(num_parse_exchanges: usize, num_inputs: usize, reject_at: usize) -> Self {
            let mut responses = VecDeque::new();
            for _ in 0..num_parse_exchanges {
                responses.push_back(frame_response(&[], 0x9000));
            }
            for i in 0..num_inputs {
                if i == reject_at {
                    // User rejection: SW 0x6985
                    responses.push_back(frame_response(&[], 0x6985));
                } else {
                    let mut sig = vec![0x30u8; 64];
                    sig.push(0x01);
                    responses.push_back(frame_response(&sig, 0x9000));
                }
            }
            Self {
                write_count: 0,
                responses,
            }
        }
    }

    impl HidIo for AutoSignHid {
        fn write_packet(&mut self, _p: &[u8; PACKET_SIZE]) -> Result<(), AppError> {
            self.write_count += 1;
            Ok(())
        }
        fn read_packet(&mut self) -> Result<[u8; PACKET_SIZE], AppError> {
            // Pop the next packet from the front response.
            if let Some(front) = self.responses.front_mut() {
                if let Some(pkt) = front.first().copied() {
                    front.remove(0);
                    if front.is_empty() {
                        self.responses.pop_front();
                    }
                    return Ok(pkt);
                }
            }
            Err(AppError::Device("mock: no more responses".into()))
        }
    }

    use crate::providers::ledger::test_helpers::frame_response;

    #[test]
    fn sign_transaction_simple_send() {
        // 1 input, 1 output (NONE covenant) — simplest possible tx.
        let plan = DraftPlan {
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
                covenant_type: COV_NONE,
                covenant_items_hex: vec![],
            }],
            change_output_index: None,
        };

        // Parse blob fits in 1 APDU (< 255 bytes), sign blob fits in 1 APDU.
        // So: 1 parse exchange + 1 sign exchange.
        let hid = AutoSignHid::new(1, 1);
        let transport = Transport::new(hid);
        let mut signer = LedgerSigner::with_transport(transport);

        // Build a test xpub from a known seed.
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).unwrap();
        let path = bip44_path(Network::Main, 0, 0, 0);
        // Account-level: derive m/44'/5353'/0'
        let account_priv = master.derive_path(&path[..3]).unwrap();
        let account_xpub = ExtendedPubKey::from_priv(&account_priv);

        let (hex, txid) =
            sign_transaction(&mut signer, &plan, &account_xpub, Network::Main, None, &[]).unwrap();

        // The tx hex should be non-empty and the txid should be 64 hex chars.
        assert!(!hex.is_empty());
        assert_eq!(txid.len(), 64);
    }

    // ---- Test helpers for the richer scenarios below ----

    /// Build a deterministic account xpub for tests.
    fn test_account_xpub() -> ExtendedPubKey {
        let seed = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        let master = ExtendedPrivKey::from_seed(&seed).unwrap();
        let path = bip44_path(Network::Main, 0, 0, 0);
        let account_priv = master.derive_path(&path[..3]).unwrap();
        ExtendedPubKey::from_priv(&account_priv)
    }

    fn plan_input(child_index: u32, branch: u32) -> crate::noncustodial::actions::PlanInput {
        crate::noncustodial::actions::PlanInput {
            txid: "aa".repeat(32),
            vout: child_index,
            value: 100_000_000,
            branch,
            child_index,
            sighash_type: 1,
        }
    }

    fn plan_output_none(value: u64) -> crate::noncustodial::actions::PlanOutput {
        crate::noncustodial::actions::PlanOutput {
            value,
            address: "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx".into(),
            covenant_type: COV_NONE,
            covenant_items_hex: vec![],
        }
    }

    /// How many parse-mode APDUs a plan produces (so tests don't hardcode
    /// fragile chunk counts).
    fn parse_exchange_count(
        plan: &DraftPlan,
        change: Option<&ChangeInfo>,
        names: &[OutputName],
    ) -> usize {
        build_parse_apdus(
            plan,
            Network::Main,
            change,
            names,
            network_flag(Network::Main),
        )
        .unwrap()
        .len()
    }

    #[test]
    fn sign_transaction_multi_input() {
        // 3 inputs, 1 output. Verify all 3 inputs get signed + witnessed.
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "main".into(),
            inputs: vec![plan_input(0, 0), plan_input(1, 0), plan_input(2, 0)],
            outputs: vec![plan_output_none(290_000_000)],
            change_output_index: None,
        };

        let parse_n = parse_exchange_count(&plan, None, &[]);
        let hid = AutoSignHid::multi_input(parse_n, 3);
        let mut signer = LedgerSigner::with_transport(Transport::new(hid));
        let account_xpub = test_account_xpub();

        let (hex, txid) =
            sign_transaction(&mut signer, &plan, &account_xpub, Network::Main, None, &[]).unwrap();

        assert!(!hex.is_empty());
        assert_eq!(txid.len(), 64);
    }

    #[test]
    fn sign_transaction_with_change() {
        // 1 input, 2 outputs (recipient + change). ChangeInfo points at index 1.
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "main".into(),
            inputs: vec![plan_input(0, 0)],
            outputs: vec![plan_output_none(60_000_000), plan_output_none(39_000_000)],
            change_output_index: Some(1),
        };
        let change = ChangeInfo {
            output_index: 1,
            address_version: 0,
            path: bip44_path(Network::Main, 0, 1, 0).to_vec(),
        };

        let parse_n = parse_exchange_count(&plan, Some(&change), &[]);
        let hid = AutoSignHid::multi_input(parse_n, 1);
        let mut signer = LedgerSigner::with_transport(Transport::new(hid));
        let account_xpub = test_account_xpub();

        let (hex, txid) = sign_transaction(
            &mut signer,
            &plan,
            &account_xpub,
            Network::Main,
            Some(&change),
            &[],
        )
        .unwrap();

        assert!(!hex.is_empty());
        assert_eq!(txid.len(), 64);
    }

    #[test]
    fn sign_transaction_name_covenant_reveal() {
        // A REVEAL covenant carries a name marker. Verify the full signing flow
        // succeeds and the name marker doesn't break parse-APDU assembly.
        use crate::noncustodial::sync::COV_REVEAL;
        // REVEAL items (hsd wire order): [nameHash, height, nonce]. The name is
        // supplied out-of-band via the OutputName marker.
        let name_hash = "bb".repeat(32);
        let height = "00000000";
        let nonce = "cc".repeat(32);
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "main".into(),
            inputs: vec![plan_input(0, 0)],
            outputs: vec![crate::noncustodial::actions::PlanOutput {
                value: 50_000_000,
                address: "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx".into(),
                covenant_type: COV_REVEAL,
                covenant_items_hex: vec![name_hash, height.into(), nonce],
            }],
            change_output_index: None,
        };
        let names = vec![OutputName {
            output_index: 0,
            name: "example".into(),
        }];

        let parse_n = parse_exchange_count(&plan, None, &names);
        let hid = AutoSignHid::multi_input(parse_n, 1);
        let mut signer = LedgerSigner::with_transport(Transport::new(hid));
        let account_xpub = test_account_xpub();

        let (hex, txid) = sign_transaction(
            &mut signer,
            &plan,
            &account_xpub,
            Network::Main,
            None,
            &names,
        )
        .unwrap();

        assert!(!hex.is_empty());
        assert_eq!(txid.len(), 64);
    }

    #[test]
    fn sign_transaction_large_multi_apdu() {
        // Many outputs push the parse blob over 255 bytes, forcing multiple
        // parse APDUs. Verify the multi-APDU parse path works end-to-end.
        let outputs: Vec<_> = (0..20).map(|_| plan_output_none(1_000_000)).collect();
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "main".into(),
            inputs: vec![plan_input(0, 0)],
            outputs,
            change_output_index: None,
        };

        let parse_n = parse_exchange_count(&plan, None, &[]);
        // Sanity: this really is a multi-APDU transaction.
        assert!(parse_n > 1, "expected multi-APDU parse, got {parse_n}");

        let hid = AutoSignHid::multi_input(parse_n, 1);
        let mut signer = LedgerSigner::with_transport(Transport::new(hid));
        let account_xpub = test_account_xpub();

        let (hex, txid) =
            sign_transaction(&mut signer, &plan, &account_xpub, Network::Main, None, &[]).unwrap();
        assert!(!hex.is_empty());
        assert_eq!(txid.len(), 64);
    }

    #[test]
    fn sign_transaction_user_rejection_mid_signing() {
        // 2 inputs; device rejects (0x6985) on the second input's signature.
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "main".into(),
            inputs: vec![plan_input(0, 0), plan_input(1, 0)],
            outputs: vec![plan_output_none(190_000_000)],
            change_output_index: None,
        };

        let parse_n = parse_exchange_count(&plan, None, &[]);
        // Reject on input index 1 (the second input).
        let hid = AutoSignHid::rejecting_at(parse_n, 2, 1);
        let mut signer = LedgerSigner::with_transport(Transport::new(hid));
        let account_xpub = test_account_xpub();

        let err = sign_transaction(&mut signer, &plan, &account_xpub, Network::Main, None, &[])
            .unwrap_err();
        // Should surface as a user-rejection error, not a panic or wrong txid.
        assert!(
            matches!(err, AppError::UserRejected),
            "expected UserRejected, got: {err:?}"
        );
    }

    #[test]
    fn build_parse_blob_rejects_too_many_outputs() {
        // >255 outputs must be rejected, not silently truncated.
        let outputs: Vec<_> = (0..256).map(|_| plan_output_none(1)).collect();
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "main".into(),
            inputs: vec![plan_input(0, 0)],
            outputs,
            change_output_index: None,
        };
        let err = build_parse_apdus(&plan, Network::Main, None, &[], network_flag(Network::Main))
            .unwrap_err();
        let msg = format!("{err:?}").to_lowercase();
        assert!(msg.contains("too large"), "expected size error, got: {msg}");
    }

    // --- Covenant signing tests (H4): one per covenant type ---

    /// Helper to build a plan with a single covenant output and run it through
    /// `sign_transaction`, asserting a valid hex + 64-char txid are produced.
    fn assert_covenant_signs(
        covenant_type: u8,
        covenant_items_hex: Vec<String>,
        name: Option<&str>,
    ) {
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "main".into(),
            inputs: vec![plan_input(0, 0)],
            outputs: vec![crate::noncustodial::actions::PlanOutput {
                value: 50_000_000,
                address: "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx".into(),
                covenant_type,
                covenant_items_hex,
            }],
            change_output_index: None,
        };
        let names: Vec<OutputName> = name
            .map(|n| {
                vec![OutputName {
                    output_index: 0,
                    name: n.to_string(),
                }]
            })
            .unwrap_or_default();

        let parse_n = parse_exchange_count(&plan, None, &names);
        let hid = AutoSignHid::multi_input(parse_n, 1);
        let mut signer = LedgerSigner::with_transport(Transport::new(hid));
        let account_xpub = test_account_xpub();

        let (hex, txid) = sign_transaction(
            &mut signer,
            &plan,
            &account_xpub,
            Network::Main,
            None,
            &names,
        )
        .unwrap();

        assert!(!hex.is_empty(), "covenant type {covenant_type}: empty hex");
        assert_eq!(
            txid.len(),
            64,
            "covenant type {covenant_type}: bad txid len"
        );
    }

    #[test]
    fn sign_transaction_covenant_open() {
        use crate::noncustodial::sync::COV_OPEN;
        // OPEN items: [nameHash, height, name, nonce]
        let name_hash = "bb".repeat(32);
        let height = "00000064"; // 100
        let name_hex = hex::encode(b"example");
        let nonce = "dd".repeat(32);
        assert_covenant_signs(
            COV_OPEN,
            vec![name_hash, height.into(), name_hex, nonce],
            None, // OPEN does not require a name marker
        );
    }

    #[test]
    fn sign_transaction_covenant_bid() {
        use crate::noncustodial::sync::COV_BID;
        // BID items: [nameHash, height, name, blind]
        let name_hash = "bb".repeat(32);
        let height = "00000064";
        let name_hex = hex::encode(b"example");
        let blind = "ee".repeat(32);
        assert_covenant_signs(
            COV_BID,
            vec![name_hash, height.into(), name_hex, blind],
            None, // BID does not require a name marker
        );
    }

    #[test]
    fn sign_transaction_covenant_redeem() {
        use crate::noncustodial::sync::COV_REDEEM;
        // REDEEM items: [nameHash, height]
        let name_hash = "bb".repeat(32);
        let height = "00000064";
        assert_covenant_signs(
            COV_REDEEM,
            vec![name_hash, height.into()],
            Some("example"), // REDEEM requires name marker
        );
    }

    #[test]
    fn sign_transaction_covenant_register() {
        use crate::noncustodial::sync::COV_REGISTER;
        // REGISTER items: [nameHash, height, data]
        let name_hash = "bb".repeat(32);
        let height = "00000064";
        let data = "ff".repeat(20); // resource data
        assert_covenant_signs(
            COV_REGISTER,
            vec![name_hash, height.into(), data],
            Some("example"), // REGISTER requires name marker
        );
    }

    #[test]
    fn sign_transaction_covenant_update() {
        use crate::noncustodial::sync::COV_UPDATE;
        // UPDATE items: [nameHash, height, data]
        let name_hash = "bb".repeat(32);
        let height = "00000064";
        let data = "ff".repeat(20);
        assert_covenant_signs(
            COV_UPDATE,
            vec![name_hash, height.into(), data],
            Some("example"), // UPDATE requires name marker
        );
    }

    #[test]
    fn sign_transaction_covenant_renew() {
        use crate::noncustodial::sync::COV_RENEW;
        // RENEW items: [nameHash, height, blockHash]
        let name_hash = "bb".repeat(32);
        let height = "00000064";
        let block_hash = "aa".repeat(32);
        assert_covenant_signs(
            COV_RENEW,
            vec![name_hash, height.into(), block_hash],
            Some("example"), // RENEW requires name marker
        );
    }

    #[test]
    fn sign_transaction_covenant_transfer() {
        use crate::noncustodial::sync::COV_TRANSFER;
        // TRANSFER items: [nameHash, height, version, address]
        let name_hash = "bb".repeat(32);
        let height = "00000064";
        let version = "00";
        let addr = "cc".repeat(20);
        assert_covenant_signs(
            COV_TRANSFER,
            vec![name_hash, height.into(), version.into(), addr],
            Some("example"), // TRANSFER requires name marker
        );
    }

    #[test]
    fn sign_transaction_covenant_finalize() {
        use crate::noncustodial::sync::COV_FINALIZE;
        // FINALIZE items: [nameHash, height, name, flags, claimHash, renewalCount, blockHash]
        let name_hash = "bb".repeat(32);
        let height = "00000064";
        let name_hex = hex::encode(b"example");
        let flags = "00";
        let claim_hash = "dd".repeat(32);
        let renewal_count = "00000001";
        let block_hash = "ee".repeat(32);
        assert_covenant_signs(
            COV_FINALIZE,
            vec![
                name_hash,
                height.into(),
                name_hex,
                flags.into(),
                claim_hash,
                renewal_count.into(),
                block_hash,
            ],
            None, // FINALIZE does not require a name marker
        );
    }

    #[test]
    fn sign_transaction_covenant_revoke() {
        use crate::noncustodial::sync::COV_REVOKE;
        // REVOKE items: [nameHash, height]
        let name_hash = "bb".repeat(32);
        let height = "00000064";
        assert_covenant_signs(
            COV_REVOKE,
            vec![name_hash, height.into()],
            Some("example"), // REVOKE requires name marker
        );
    }

    /// H1 invariant: the txid returned by `sign_transaction` must equal the
    /// txid of the independently-rebuilt unsigned transaction. `sign_via_ledger`
    /// relies on this to verify the device signed the previewed tx (Handshake
    /// txids hash the non-witness serialization only, so signatures can't move
    /// the txid). If this ever diverges, the covenant-txid guard would reject
    /// good signatures — or worse, a silent divergence would go undetected.
    #[test]
    fn signed_txid_matches_rebuilt_unsigned_txid() {
        use crate::noncustodial::sync::COV_TRANSFER;
        let plan = DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "main".into(),
            inputs: vec![plan_input(0, 0), plan_input(1, 0)],
            outputs: vec![crate::noncustodial::actions::PlanOutput {
                value: 50_000_000,
                address: "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx".into(),
                covenant_type: COV_TRANSFER,
                covenant_items_hex: vec![
                    "bb".repeat(32),
                    "00000064".into(),
                    "00".into(),
                    "cc".repeat(20),
                ],
            }],
            change_output_index: None,
        };
        let names = vec![OutputName {
            output_index: 0,
            name: "example".into(),
        }];

        // Independently rebuild the unsigned tx and take its txid — this is the
        // "expected" value the command layer compares against.
        let expected_txid = crate::noncustodial::actions::rebuild_unsigned(&plan, Network::Main)
            .unwrap()
            .txid();

        let parse_n = parse_exchange_count(&plan, None, &names);
        let hid = AutoSignHid::multi_input(parse_n, 2);
        let mut signer = LedgerSigner::with_transport(Transport::new(hid));
        let account_xpub = test_account_xpub();

        let (_hex, signed_txid) = sign_transaction(
            &mut signer,
            &plan,
            &account_xpub,
            Network::Main,
            None,
            &names,
        )
        .unwrap();

        assert_eq!(
            signed_txid, expected_txid,
            "signed txid must match the rebuilt unsigned txid (H1 invariant)"
        );
    }

    // --- M7: device-error parity across the signing flow ---
    //
    // The parse phase and the sign phase both funnel through `exchange_ok`,
    // which maps status words to typed errors. These tests assert that a
    // device fault surfaces the right `AppError` variant regardless of *which*
    // phase produced it — so the frontend renders the same actionable guidance
    // whether the device is locked/wrong-app/rejected during parse or sign.

    /// A mock that returns SW `0x9000` for every exchange except exchange
    /// number `fail_at` (0-based across the whole parse+sign sequence), which
    /// returns `fail_sw`.
    struct FailAtHid {
        exchange_idx: usize,
        fail_at: usize,
        fail_sw: u16,
        pending: VecDeque<[u8; PACKET_SIZE]>,
    }

    impl FailAtHid {
        fn new(fail_at: usize, fail_sw: u16) -> Self {
            Self {
                exchange_idx: 0,
                fail_at,
                fail_sw,
                pending: VecDeque::new(),
            }
        }
    }

    impl HidIo for FailAtHid {
        fn write_packet(&mut self, _p: &[u8; PACKET_SIZE]) -> Result<(), AppError> {
            // A write starts a new exchange: decide its response now.
            let sw = if self.exchange_idx == self.fail_at {
                self.fail_sw
            } else {
                0x9000
            };
            // On a success SW during the sign phase we don't know here whether
            // the caller expects a signature body; but every failing test aborts
            // at `fail_at` before reaching a real signature, and the success
            // branches are already covered by the AutoSignHid tests. So an empty
            // body on success is sufficient for the fault-injection path.
            let body: &[u8] = &[];
            for pkt in frame_response(body, sw) {
                self.pending.push_back(pkt);
            }
            self.exchange_idx += 1;
            Ok(())
        }
        fn read_packet(&mut self) -> Result<[u8; PACKET_SIZE], AppError> {
            self.pending
                .pop_front()
                .ok_or_else(|| AppError::Device("mock: no more responses".into()))
        }
    }

    fn one_input_plan() -> DraftPlan {
        DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "main".into(),
            inputs: vec![plan_input(0, 0)],
            outputs: vec![plan_output_none(99_000_000)],
            change_output_index: None,
        }
    }

    /// Run a single-input plan against a device that fails at `fail_at` with
    /// `fail_sw`, returning the resulting error.
    fn sign_with_fault(fail_at: usize, fail_sw: u16) -> AppError {
        let plan = one_input_plan();
        let hid = FailAtHid::new(fail_at, fail_sw);
        let mut signer = LedgerSigner::with_transport(Transport::new(hid));
        let account_xpub = test_account_xpub();
        sign_transaction(&mut signer, &plan, &account_xpub, Network::Main, None, &[]).unwrap_err()
    }

    #[test]
    fn parse_phase_user_rejection_maps_to_user_rejected() {
        // Reject on the very first (parse) exchange — the existing rejection
        // test only covers the sign phase.
        let err = sign_with_fault(0, 0x6985);
        assert!(
            matches!(err, AppError::UserRejected),
            "parse-phase 0x6985 should be UserRejected, got: {err:?}"
        );
    }

    #[test]
    fn parse_phase_locked_device_maps_to_device_error() {
        let err = sign_with_fault(0, 0x5515);
        match err {
            AppError::Device(msg) => {
                assert!(msg.contains("0x5515"), "expected SW in message, got: {msg}");
                assert!(msg.contains("locked"), "expected 'locked' hint, got: {msg}");
            }
            other => panic!("expected Device error for locked device, got: {other:?}"),
        }
    }

    #[test]
    fn parse_phase_wrong_app_maps_to_device_error() {
        // 0x6d00 = instruction not supported (Handshake app not open).
        let err = sign_with_fault(0, 0x6d00);
        match err {
            AppError::Device(msg) => {
                assert!(msg.contains("0x6d00"), "expected SW in message, got: {msg}");
                assert!(msg.contains("app"), "expected app hint, got: {msg}");
            }
            other => panic!("expected Device error for wrong app, got: {other:?}"),
        }
    }

    #[test]
    fn parse_phase_class_not_supported_maps_to_device_error() {
        // 0x6e00 = class not supported (wrong app open).
        let err = sign_with_fault(0, 0x6e00);
        match err {
            AppError::Device(msg) => assert!(msg.contains("0x6e00"), "got: {msg}"),
            other => panic!("expected Device error carrying 0x6e00, got: {other:?}"),
        }
    }

    #[test]
    fn parse_phase_invalid_data_maps_to_device_error() {
        // 0x6a80 = device rejected the covenant/tx serialization.
        let err = sign_with_fault(0, 0x6a80);
        match err {
            AppError::Device(msg) => assert!(msg.contains("0x6a80"), "got: {msg}"),
            other => panic!("expected Device error carrying 0x6a80, got: {other:?}"),
        }
    }

    #[test]
    fn sign_phase_locked_device_maps_to_device_error() {
        // For a 1-input plan there is exactly 1 parse exchange, so exchange
        // index 1 is the (only) sign exchange. A fault there must surface the
        // same Device error as during parse — proving phase parity.
        let parse_n = parse_exchange_count(&one_input_plan(), None, &[]);
        let err = sign_with_fault(parse_n, 0x5515);
        match err {
            AppError::Device(msg) => assert!(msg.contains("0x5515"), "got: {msg}"),
            other => panic!("sign-phase locked device should be Device, got: {other:?}"),
        }
    }

    #[test]
    fn sign_phase_user_rejection_maps_to_user_rejected() {
        let parse_n = parse_exchange_count(&one_input_plan(), None, &[]);
        let err = sign_with_fault(parse_n, 0x6985);
        assert!(
            matches!(err, AppError::UserRejected),
            "sign-phase 0x6985 should be UserRejected, got: {err:?}"
        );
    }
}
