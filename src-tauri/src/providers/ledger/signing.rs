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
use crate::noncustodial::tx::{output_address_from_string, Covenant, Input, Outpoint, Output, Transaction};

use super::apdu::network_flag;
use super::hid_transport::HidIo;
use super::parse_mode::{build_parse_apdus, ChangeInfo, OutputName};
use super::sign_mode::{build_sign_apdus, parse_signature};
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

        let sign_cmds = build_sign_apdus(
            network,
            plan.account,
            inp.branch,
            inp.child_index,
            inp.sighash_type,
            &prevout_hash,
            inp.vout,
            inp.value,
            0xFFFF_FFFF, // sequence (always final)
            &pubkey,
        )?;

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
    let mut tx = rebuild_unsigned_tx(plan, network)?;
    for (i, inp) in plan.inputs.iter().enumerate() {
        let child = account_xpub.derive_path(&[inp.branch, inp.child_index])?;
        let pubkey = child.compressed_pubkey();
        // Witness: [signature, pubkey]
        tx.inputs[i].witness = vec![signatures[i].to_vec(), pubkey.to_vec()];
    }

    Ok((tx.to_hex(), tx.txid()))
}

/// Rebuild the unsigned transaction from a plan (same as `actions::rebuild_unsigned`
/// but accessible from the ledger module without coupling to private internals).
fn rebuild_unsigned_tx(plan: &DraftPlan, network: Network) -> Result<Transaction, AppError> {
    let mut tx = Transaction::new();
    tx.version = plan.version;
    tx.locktime = plan.locktime;

    for inp in &plan.inputs {
        let hash = hex_to_32(&inp.txid)?;
        tx.inputs.push(Input::new(Outpoint {
            hash,
            index: inp.vout,
        }));
    }

    for out in &plan.outputs {
        let items: Vec<Vec<u8>> = out
            .covenant_items_hex
            .iter()
            .map(|h| {
                hex::decode(h)
                    .map_err(|e| AppError::InvalidInput(format!("bad covenant item hex: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        tx.outputs.push(Output {
            value: out.value,
            address: output_address_from_string(network, &out.address)?,
            covenant: Covenant {
                covenant_type: out.covenant_type,
                items,
            },
        });
    }

    Ok(tx)
}

fn hex_to_32(hex_str: &str) -> Result<[u8; 32], AppError> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| AppError::InvalidInput(format!("bad txid hex: {e}")))?;
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

        let (hex, txid) = sign_transaction(
            &mut signer,
            &plan,
            &account_xpub,
            Network::Main,
            None,
            &[],
        )
        .unwrap();

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
        build_parse_apdus(plan, Network::Main, change, names, network_flag(Network::Main))
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
}
