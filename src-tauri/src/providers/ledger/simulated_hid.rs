//! In-process Ledger device **simulator** for manual UI testing.
//!
//! Compiled only under the dev-only `mock-ledger` Cargo feature. When
//! [`LedgerSigner::connect`](super::LedgerSigner::connect) sees the
//! `NAMEHOLD_LEDGER_SIM` environment variable, it instantiates a
//! [`SimulatedHid`] instead of opening real hardware, letting you click
//! through every Ledger UX path without a physical device:
//!
//! ```text
//! NAMEHOLD_LEDGER_SIM=happy      cargo tauri dev --features mock-ledger
//! NAMEHOLD_LEDGER_SIM=no_device  …
//! NAMEHOLD_LEDGER_SIM=wrong_app  …
//! NAMEHOLD_LEDGER_SIM=locked     …
//! NAMEHOLD_LEDGER_SIM=reject     …
//! NAMEHOLD_LEDGER_SIM=timeout    …
//! NAMEHOLD_LEDGER_SIM=disconnect …
//! ```
//!
//! ## How it works
//!
//! The simulator sits at the [`HidIo`] boundary, exchanging already-HID-framed
//! 64-byte packets. It reassembles each incoming APDU (to read the `INS`
//! opcode and count how many sign requests have happened), then queues the
//! appropriate framed response for the reader to drain — exactly mirroring how
//! a real device streams frames back through [`Transport`](super::hid_transport).

use std::collections::VecDeque;

use crate::error::AppError;

use super::apdu::{
    INS_GET_APP_VERSION, INS_GET_INPUT_SIGNATURE, INS_GET_PUBLIC_KEY, SW_OK, SW_USER_REJECTED,
};
use super::hid_transport::{HidIo, PACKET_SIZE};
use super::test_helpers::frame_response;

/// Ledger status word: instruction not supported (wrong app open).
const SW_INS_NOT_SUPPORTED: u16 = 0x6d00;
/// Ledger status word: device is locked.
const SW_DEVICE_LOCKED: u16 = 0x5515;

/// Which failure/success behaviour the simulator exhibits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimMode {
    /// Every APDU succeeds; import + signing complete normally.
    Happy,
    /// `connect()` reports no device found (handled before the transport).
    NoDevice,
    /// First APDU answers 0x6d00 — the Handshake app isn't open.
    WrongApp,
    /// First APDU answers 0x5515 — the device is locked.
    Locked,
    /// Parse phase succeeds; the first signature request is rejected (0x6985).
    Reject,
    /// Parse phase succeeds; the first signature request never returns
    /// (mimics the 30s device timeout → 0-byte read).
    Timeout,
    /// Parse phase succeeds; the device vanishes mid-signing (HID read error).
    Disconnect,
}

impl SimMode {
    /// Parse the `NAMEHOLD_LEDGER_SIM` value into a mode.
    pub fn parse(s: &str) -> Result<Self, AppError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "happy" | "" => Ok(Self::Happy),
            "no_device" | "none" | "missing" => Ok(Self::NoDevice),
            "wrong_app" | "wrongapp" => Ok(Self::WrongApp),
            "locked" => Ok(Self::Locked),
            "reject" | "rejected" => Ok(Self::Reject),
            "timeout" => Ok(Self::Timeout),
            "disconnect" | "unplug" => Ok(Self::Disconnect),
            other => Err(AppError::Device(format!(
                "unknown NAMEHOLD_LEDGER_SIM mode '{other}' (expected one of: \
                 happy, no_device, wrong_app, locked, reject, timeout, disconnect)"
            ))),
        }
    }
}

/// A simulated Ledger HID device. See the module docs.
pub struct SimulatedHid {
    mode: SimMode,
    /// Reassembly buffer for the APDU currently being written to us.
    in_buf: Vec<u8>,
    /// Total length of the in-flight APDU (from the first frame), if known.
    in_total: Option<usize>,
    /// Queued response frames waiting to be read back.
    out_frames: VecDeque<[u8; PACKET_SIZE]>,
    /// Set once we've decided this exchange should time out; the next read hangs.
    pending_timeout: bool,
    /// Set once we've decided this exchange should disconnect; the next read errors.
    pending_disconnect: bool,
}

impl SimulatedHid {
    pub fn new(mode: SimMode) -> Self {
        Self {
            mode,
            in_buf: Vec::new(),
            in_total: None,
            out_frames: VecDeque::new(),
            pending_timeout: false,
            pending_disconnect: false,
        }
    }

    /// A full APDU has been reassembled; decide what to respond with and queue
    /// the framed response (or arm a timeout/disconnect).
    fn handle_apdu(&mut self, apdu: &[u8]) {
        // Raw APDU layout: CLA | INS | P1 | P2 | Lc | data...
        let ins = apdu.get(1).copied().unwrap_or(0);

        // Modes that fail on the very first APDU regardless of type.
        match self.mode {
            SimMode::WrongApp => {
                self.queue(&[], SW_INS_NOT_SUPPORTED);
                return;
            }
            SimMode::Locked => {
                self.queue(&[], SW_DEVICE_LOCKED);
                return;
            }
            _ => {}
        }

        match ins {
            INS_GET_APP_VERSION => {
                // Report version 1.6.0 (satisfies the >=1.6 check in import).
                self.queue(&[1, 6, 0], SW_OK);
            }
            INS_GET_PUBLIC_KEY => {
                // pubkey[33] | ccLen(32) | cc[32] | fpLen(0) | (no addr)
                let mut body = Vec::with_capacity(33 + 1 + 32 + 1);
                // A deterministic, valid-length compressed pubkey (0x02 prefix).
                body.push(0x02);
                body.extend(std::iter::repeat_n(0x11, 32));
                body.push(32); // chain-code length
                body.extend(std::iter::repeat_n(0x22, 32));
                body.push(0); // fingerprint length
                self.queue(&body, SW_OK);
            }
            INS_GET_INPUT_SIGNATURE => {
                self.handle_signature_apdu(apdu);
            }
            _ => {
                // Unknown instruction — behave like a well-behaved device and ack.
                self.queue(&[], SW_OK);
            }
        }
    }

    /// Handle a `GET_INPUT_SIGNATURE` APDU. This one instruction drives both
    /// phases, distinguished by P2:
    ///   * **P2 == 0x00** → parse mode: the device accumulates state and
    ///     answers with an empty body + 0x9000.
    ///   * **P2 == 0x01** → sign mode: for standard p2wpkh inputs this is a
    ///     single APDU that returns a 65-byte compact signature. (Multi-APDU
    ///     sign sequences — only for exotic scripts — aren't exercised by the
    ///     simulator's click-through paths.)
    ///
    /// In `reject`/`timeout`/`disconnect` modes, the *first* sign-mode APDU is
    /// where the failure is injected.
    fn handle_signature_apdu(&mut self, apdu: &[u8]) {
        let p2 = apdu.get(3).copied().unwrap_or(0);
        let is_sign_phase = p2 == 0x01;

        if !is_sign_phase {
            // Parse phase: acknowledge with empty OK.
            self.queue(&[], SW_OK);
            return;
        }

        // Signature-bearing request. Apply the failure mode (once).
        match self.mode {
            SimMode::Reject => self.queue(&[], SW_USER_REJECTED),
            SimMode::Timeout => self.pending_timeout = true,
            SimMode::Disconnect => self.pending_disconnect = true,
            _ => {
                // Happy path: dummy 65-byte compact signature
                // (r[32] || s[32] || sighashType[1]).
                let mut sig = vec![0x42u8; 64];
                sig.push(0x01); // sighash type ALL
                self.queue(&sig, SW_OK);
            }
        }
    }

    /// Frame a `body + sw` response and enqueue it for reading.
    fn queue(&mut self, body: &[u8], sw: u16) {
        for frame in frame_response(body, sw) {
            self.out_frames.push_back(frame);
        }
    }
}

impl HidIo for SimulatedHid {
    fn write_packet(&mut self, packet: &[u8; PACKET_SIZE]) -> Result<(), AppError> {
        // Reassemble the framed APDU. Frame layout mirrors Transport::frame:
        //   channel(2) | tag(1) | seq(2) | [totalLen(2) if seq==0] | payload
        let seq = u16::from_be_bytes([packet[3], packet[4]]);
        let payload_start = if seq == 0 {
            self.in_total = Some(u16::from_be_bytes([packet[5], packet[6]]) as usize);
            self.in_buf.clear();
            7
        } else {
            5
        };

        let total = self.in_total.unwrap_or(0);
        let remaining = total.saturating_sub(self.in_buf.len());
        let avail = PACKET_SIZE - payload_start;
        let take = remaining.min(avail);
        self.in_buf
            .extend_from_slice(&packet[payload_start..payload_start + take]);

        // Full APDU received → produce its response.
        if self.in_buf.len() >= total && total > 0 {
            let apdu = std::mem::take(&mut self.in_buf);
            self.in_total = None;
            self.handle_apdu(&apdu);
        }
        Ok(())
    }

    fn read_packet(&mut self) -> Result<[u8; PACKET_SIZE], AppError> {
        if self.pending_disconnect {
            return Err(AppError::Device(
                "HID read failed: device disconnected".into(),
            ));
        }
        if self.pending_timeout {
            // Mimic the real 30s device timeout: sleep, then report 0-byte read.
            std::thread::sleep(std::time::Duration::from_secs(31));
            return Err(AppError::Device(
                "timed out waiting for the Ledger — approve or reject the prompt on the device"
                    .into(),
            ));
        }
        self.out_frames.pop_front().ok_or_else(|| {
            AppError::Device("mock: no queued response (unexpected read)".into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ledger::apdu::ApduCommand;
    use crate::providers::ledger::hid_transport::Transport;
    use crate::providers::ledger::LedgerSigner;

    fn signer(mode: SimMode) -> LedgerSigner<SimulatedHid> {
        let io = SimulatedHid::new(mode);
        LedgerSigner::with_transport(Transport::new(io))
    }

    #[test]
    fn happy_get_app_version() {
        let mut s = signer(SimMode::Happy);
        let v = s.get_app_version().unwrap();
        assert_eq!((v.major, v.minor, v.patch), (1, 6, 0));
    }

    #[test]
    fn happy_get_public_key() {
        let mut s = signer(SimMode::Happy);
        let (pk, cc) = s
            .get_account_pubkey(
                crate::noncustodial::network::Network::Main,
                0,
                false,
            )
            .unwrap();
        // Deterministic pubkey from the sim.
        assert_eq!(pk[0], 0x02);
        assert_eq!(pk[1..], [0x11u8; 32]);
        assert_eq!(cc, [0x22u8; 32]);
    }

    #[test]
    fn wrong_app_fails_immediately() {
        let mut s = signer(SimMode::WrongApp);
        let err = s.get_app_version().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("6d00") || msg.contains("not supported"), "got: {msg}");
    }

    #[test]
    fn locked_fails_immediately() {
        let mut s = signer(SimMode::Locked);
        let err = s.get_app_version().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("5515") || msg.contains("locked"), "got: {msg}");
    }

    #[test]
    fn reject_on_sign_phase() {
        let mut s = signer(SimMode::Reject);
        // Version check succeeds.
        assert!(s.get_app_version().is_ok());
        // Simulate a parse-mode APDU (P2=0x00) — should succeed.
        let parse_cmd = ApduCommand {
            cla: 0xE0,
            ins: INS_GET_INPUT_SIGNATURE,
            p1: 0x01,
            p2: 0x00,
            data: vec![0u8; 10],
        };
        let result = s.transport_mut().exchange_ok(&parse_cmd);
        assert!(result.is_ok(), "parse should succeed: {:?}", result);
        // Simulate a sign-mode APDU (P2=0x01) — should be rejected.
        let sign_cmd = ApduCommand {
            cla: 0xE0,
            ins: INS_GET_INPUT_SIGNATURE,
            p1: 0x01,
            p2: 0x01,
            data: vec![0u8; 10],
        };
        let err = s.transport_mut().exchange_ok(&sign_cmd).unwrap_err();
        assert!(
            matches!(err, AppError::UserRejected),
            "expected UserRejected, got: {err:?}"
        );
    }

    #[test]
    fn disconnect_on_sign_phase() {
        let mut s = signer(SimMode::Disconnect);
        assert!(s.get_app_version().is_ok());
        // Parse OK.
        let parse_cmd = ApduCommand {
            cla: 0xE0,
            ins: INS_GET_INPUT_SIGNATURE,
            p1: 0x01,
            p2: 0x00,
            data: vec![0u8; 10],
        };
        assert!(s.transport_mut().exchange_ok(&parse_cmd).is_ok());
        // Sign → disconnect.
        let sign_cmd = ApduCommand {
            cla: 0xE0,
            ins: INS_GET_INPUT_SIGNATURE,
            p1: 0x01,
            p2: 0x01,
            data: vec![0u8; 10],
        };
        let err = s.transport_mut().exchange_ok(&sign_cmd).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("disconnected"), "got: {msg}");
    }

    #[test]
    fn sim_mode_parse() {
        assert_eq!(SimMode::parse("happy").unwrap(), SimMode::Happy);
        assert_eq!(SimMode::parse("REJECT").unwrap(), SimMode::Reject);
        assert_eq!(SimMode::parse("no_device").unwrap(), SimMode::NoDevice);
        assert_eq!(SimMode::parse("wrong_app").unwrap(), SimMode::WrongApp);
        assert_eq!(SimMode::parse("locked").unwrap(), SimMode::Locked);
        assert_eq!(SimMode::parse("timeout").unwrap(), SimMode::Timeout);
        assert_eq!(SimMode::parse("disconnect").unwrap(), SimMode::Disconnect);
        assert!(SimMode::parse("bogus").is_err());
    }
}
