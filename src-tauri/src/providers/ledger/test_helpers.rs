//! Shared test helpers for the Ledger module tests.
//!
//! Provides `frame_response()` — builds HID response packets from a body and
//! status word, supporting multi-frame responses for large payloads.

use super::hid_transport::PACKET_SIZE;

/// Build HID response frame(s) for a given body + status word.
///
/// Supports multi-frame responses: if `body + SW` exceeds the single-frame
/// payload capacity, it splits across multiple 64-byte HID packets with
/// sequential sequence numbers.
pub fn frame_response(body: &[u8], sw: u16) -> Vec<[u8; PACKET_SIZE]> {
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
