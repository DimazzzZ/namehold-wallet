//! Ledger HID transport: device discovery + the 0x0101/0x05 packet framing
//! that wraps every APDU, layered on top of the cross-platform `hidapi` crate.
//!
//! Framing (hsd-ledger `lib/apdu/io.js`), 64-byte HID packets:
//!
//! ```text
//! First frame:  channel(0x0101) | tag(0x05) | seq(0) | totalLen | payload[..57]
//! Continuation: channel(0x0101) | tag(0x05) | seq(n) | payload[..59]
//! ```
//!
//! The reader mirrors this: reassemble frames until `totalLen` bytes are
//! collected; the final two bytes of the reassembled buffer are the status
//! word (SW1 SW2, big-endian).

use crate::error::AppError;
use crate::providers::ledger::apdu::{ApduCommand, SW_OK, SW_USER_REJECTED};

/// Ledger USB vendor id (`0x2C97`), shared across Nano S / S Plus / X.
pub const LEDGER_VENDOR_ID: u16 = 0x2C97;

/// HID packet size for Ledger devices.
pub const PACKET_SIZE: usize = 64;

const CHANNEL_ID: u16 = 0x0101;
const TAG_APDU: u8 = 0x05;

/// Abstraction over the raw HID read/write so tests can substitute a mock or a
/// Speculos-backed transport without a physical device.
///
/// Implementors exchange **already-HID-framed** 64-byte packets. Framing and
/// APDU assembly live in [`Transport::exchange`], which is provided.
pub trait HidIo {
    /// Write one 64-byte HID packet (report-id prefix handled by the impl).
    fn write_packet(&mut self, packet: &[u8; PACKET_SIZE]) -> Result<(), AppError>;
    /// Read one 64-byte HID packet, blocking up to the transport's timeout.
    fn read_packet(&mut self) -> Result<[u8; PACKET_SIZE], AppError>;
}

/// A full APDU exchange built on top of any [`HidIo`].
pub struct Transport<T: HidIo> {
    io: T,
}

impl<T: HidIo> Transport<T> {
    pub fn new(io: T) -> Self {
        Self { io }
    }

    /// Frame `raw` (a serialized APDU) into 64-byte HID packets.
    fn frame(raw: &[u8]) -> Vec<[u8; PACKET_SIZE]> {
        let mut packets = Vec::new();
        let mut offset = 0usize;
        let mut seq: u16 = 0;

        while offset < raw.len() || seq == 0 {
            let mut pkt = [0u8; PACKET_SIZE];
            pkt[0..2].copy_from_slice(&CHANNEL_ID.to_be_bytes());
            pkt[2] = TAG_APDU;
            pkt[3..5].copy_from_slice(&seq.to_be_bytes());

            let mut header_len = 5;
            if seq == 0 {
                // First frame carries the total length.
                pkt[5..7].copy_from_slice(&(raw.len() as u16).to_be_bytes());
                header_len = 7;
            }

            let space = PACKET_SIZE - header_len;
            let end = (offset + space).min(raw.len());
            let chunk = &raw[offset..end];
            pkt[header_len..header_len + chunk.len()].copy_from_slice(chunk);
            packets.push(pkt);

            offset = end;
            seq += 1;

            // For a zero-length APDU (never happens here) we still emit one.
            if raw.is_empty() {
                break;
            }
        }
        packets
    }

    /// Send one APDU command and return `(response_body, status_word)`.
    /// `response_body` excludes the 2-byte status word.
    pub fn exchange(&mut self, cmd: &ApduCommand) -> Result<(Vec<u8>, u16), AppError> {
        let raw = cmd.to_raw()?;
        for pkt in Self::frame(&raw) {
            self.io.write_packet(&pkt)?;
        }

        // Reassemble the response.
        let mut total_len: Option<usize> = None;
        let mut buf: Vec<u8> = Vec::new();
        let mut expected_seq: u16 = 0;

        loop {
            let pkt = self.io.read_packet()?;
            let channel = u16::from_be_bytes([pkt[0], pkt[1]]);
            if channel != CHANNEL_ID {
                return Err(AppError::Device(format!(
                    "unexpected HID channel 0x{channel:04x}"
                )));
            }
            if pkt[2] != TAG_APDU {
                return Err(AppError::Device(format!(
                    "unexpected HID tag 0x{:02x}",
                    pkt[2]
                )));
            }
            let seq = u16::from_be_bytes([pkt[3], pkt[4]]);
            if seq != expected_seq {
                return Err(AppError::Device(format!(
                    "HID sequence out of order: got {seq}, expected {expected_seq}"
                )));
            }

            let payload_start = if seq == 0 {
                total_len = Some(u16::from_be_bytes([pkt[5], pkt[6]]) as usize);
                7
            } else {
                5
            };

            let want = total_len.ok_or_else(|| {
                AppError::Device("HID response missing length header".into())
            })?;
            let remaining = want.saturating_sub(buf.len());
            let avail = PACKET_SIZE - payload_start;
            let take = remaining.min(avail);
            buf.extend_from_slice(&pkt[payload_start..payload_start + take]);
            expected_seq += 1;

            if buf.len() >= want {
                break;
            }
        }

        if buf.len() < 2 {
            return Err(AppError::Device(
                "APDU response shorter than a status word".into(),
            ));
        }
        let sw = u16::from_be_bytes([buf[buf.len() - 2], buf[buf.len() - 1]]);
        buf.truncate(buf.len() - 2);
        Ok((buf, sw))
    }

    /// Like [`exchange`], but treats any non-`0x9000` status word as an error,
    /// mapping the well-known "user rejected" code to [`AppError::UserRejected`].
    pub fn exchange_ok(&mut self, cmd: &ApduCommand) -> Result<Vec<u8>, AppError> {
        let (body, sw) = self.exchange(cmd)?;
        match sw {
            SW_OK => Ok(body),
            SW_USER_REJECTED => Err(AppError::UserRejected),
            other => Err(AppError::Device(status_word_message(other))),
        }
    }

    /// Consume the transport, returning the inner IO (useful for reuse/tests).
    pub fn into_inner(self) -> T {
        self.io
    }
}

/// Human-readable message for a non-success status word.
pub fn status_word_message(sw: u16) -> String {
    let hint = match sw {
        0x6985 => " (user rejected on device)",
        0x6d00 => " (instruction not supported — is the Handshake app open?)",
        0x6e00 => " (class not supported — wrong app open?)",
        0x6a80 => " (invalid data — covenant/tx serialization rejected)",
        0x5515 => " (device locked — unlock it)",
        _ => "",
    };
    format!("APDU failed with status 0x{sw:04x}{hint}")
}

// --- Real hidapi-backed IO -------------------------------------------------

/// A physical Ledger HID connection via the `hidapi` crate.
pub struct RealHid {
    device: hidapi::HidDevice,
}

impl RealHid {
    /// Open the first connected Ledger device. Returns [`AppError::Device`]
    /// with actionable guidance when no device is found.
    pub fn open_first() -> Result<Self, AppError> {
        let api = hidapi::HidApi::new()
            .map_err(|e| AppError::Device(format!("HID init failed: {e}")))?;
        let info = api
            .device_list()
            .find(|d| d.vendor_id() == LEDGER_VENDOR_ID && usable_interface(d))
            .ok_or_else(|| {
                AppError::Device(
                    "no Ledger device found — plug it in, unlock it, and open the Handshake app"
                        .into(),
                )
            })?;
        let path = info.path().to_owned();
        let device = api
            .open_path(&path)
            .map_err(|e| AppError::Device(format!("failed to open Ledger HID device: {e}")))?;
        Ok(Self { device })
    }
}

/// Whether a HID interface entry is the one we can talk APDUs on. On
/// macOS/Windows Ledger exposes usage page 0xFFA0; on Linux we match interface 0.
fn usable_interface(d: &hidapi::DeviceInfo) -> bool {
    d.usage_page() == 0xFFA0 || d.interface_number() == 0
}

impl HidIo for RealHid {
    fn write_packet(&mut self, packet: &[u8; PACKET_SIZE]) -> Result<(), AppError> {
        // hidapi expects a leading report-id byte (0x00) on write.
        let mut framed = [0u8; PACKET_SIZE + 1];
        framed[1..].copy_from_slice(packet);
        self.device
            .write(&framed)
            .map_err(|e| AppError::Device(format!("HID write failed: {e}")))?;
        Ok(())
    }

    fn read_packet(&mut self) -> Result<[u8; PACKET_SIZE], AppError> {
        let mut buf = [0u8; PACKET_SIZE];
        // 30s timeout: on-device confirmation of a tx can take a while.
        let n = self
            .device
            .read_timeout(&mut buf, 30_000)
            .map_err(|e| AppError::Device(format!("HID read failed: {e}")))?;
        if n == 0 {
            return Err(AppError::Device(
                "timed out waiting for the Ledger — approve or reject the prompt on the device"
                    .into(),
            ));
        }
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A scripted mock: caller pre-loads the exact 64-byte packets the device
    /// would return; writes are recorded for assertions.
    pub struct MockHid {
        pub writes: Vec<[u8; PACKET_SIZE]>,
        pub reads: VecDeque<[u8; PACKET_SIZE]>,
    }

    impl HidIo for MockHid {
        fn write_packet(&mut self, packet: &[u8; PACKET_SIZE]) -> Result<(), AppError> {
            self.writes.push(*packet);
            Ok(())
        }
        fn read_packet(&mut self) -> Result<[u8; PACKET_SIZE], AppError> {
            self.reads
                .pop_front()
                .ok_or_else(|| AppError::Device("mock: no more read packets".into()))
        }
    }

    use crate::providers::ledger::test_helpers::frame_response;

    #[test]
    fn roundtrip_short_response() {
        use crate::providers::ledger::apdu::get_app_version;
        let reads = frame_response(&[0x00, 0x01, 0x02], SW_OK);
        let mock = MockHid {
            writes: Vec::new(),
            reads: reads.into(),
        };
        let mut t = Transport::new(mock);
        let body = t.exchange_ok(&get_app_version()).unwrap();
        assert_eq!(body, vec![0x00, 0x01, 0x02]);
        // One write packet for the empty-payload command.
        let io = t.into_inner();
        assert_eq!(io.writes.len(), 1);
        assert_eq!(&io.writes[0][0..3], &[0x01, 0x01, 0x05]);
    }

    #[test]
    fn multi_frame_response_reassembles() {
        use crate::providers::ledger::apdu::get_app_version;
        // 200-byte body forces continuation frames on the read side.
        let body: Vec<u8> = (0..200).map(|i| (i % 256) as u8).collect();
        let reads = frame_response(&body, SW_OK);
        assert!(reads.len() >= 4, "should span multiple frames");
        let mock = MockHid {
            writes: Vec::new(),
            reads: reads.into(),
        };
        let mut t = Transport::new(mock);
        let got = t.exchange_ok(&get_app_version()).unwrap();
        assert_eq!(got, body);
    }

    #[test]
    fn user_rejected_maps_to_error() {
        use crate::providers::ledger::apdu::get_app_version;
        let reads = frame_response(&[], SW_USER_REJECTED);
        let mock = MockHid {
            writes: Vec::new(),
            reads: reads.into(),
        };
        let mut t = Transport::new(mock);
        let err = t.exchange_ok(&get_app_version()).unwrap_err();
        assert!(matches!(err, AppError::UserRejected));
    }
}
