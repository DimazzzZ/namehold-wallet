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

    fn frame_response(body: &[u8], sw: u16) -> Vec<[u8; PACKET_SIZE]> {
        let mut raw = body.to_vec();
        raw.extend_from_slice(&sw.to_be_bytes());
        let mut packets = Vec::new();
        let mut offset = 0usize;
        let mut seq: u16 = 0;
        while offset < raw.len() || seq == 0 {
            let mut pkt = [0u8; PACKET_SIZE];
            pkt[0..2].copy_from_slice(&0x0101u16.to_be_bytes());
            pkt[2] = 0x05;
            pkt[3..5].copy_from_slice(&seq.to_be_bytes());
            let header_len = if seq == 0 {
                pkt[5..7].copy_from_slice(&(raw.len() as u16).to_be_bytes());
                7
            } else {
                5
            };
            let space = PACKET_SIZE - header_len;
            let end = (offset + space).min(raw.len());
            pkt[header_len..header_len + (end - offset)].copy_from_slice(&raw[offset..end]);
            packets.push(pkt);
            offset = end;
            seq += 1;
            if raw.is_empty() {
                break;
            }
        }
        packets
    }

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
}
