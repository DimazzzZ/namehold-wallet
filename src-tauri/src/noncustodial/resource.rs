//! Handshake DNS `Resource` encode/decode, verified against hsd v6.1.1
//! `lib/dns/resource.js` + `lib/dns/common.js`.
//!
//! Wire format: `u8(version=0)` then, for each record, `u8(type)` + the
//! record body, read until end-of-buffer (no count prefix). Record type bytes
//! (`hsTypes`): DS=0, NS=1, GLUE4=2, GLUE6=3, SYNTH4=4, SYNTH6=5, TXT=6.
//!
//! Domain names use the bns label encoding (`u8(len)+label …` terminated by a
//! 0 byte). We emit them UNCOMPRESSED, which hsd's reader accepts.
//!
//! Records are exchanged with the frontend as JSON objects matching Bob
//! Wallet's shape, e.g. `{"type":"TXT","txt":["hello"]}`,
//! `{"type":"NS","ns":"ns1.example."}`,
//! `{"type":"DS","keyTag":1,"algorithm":8,"digestType":2,"digest":"<hex>"}`,
//! `{"type":"GLUE4","ns":"ns1.example.","address":"1.2.3.4"}`,
//! `{"type":"SYNTH6","address":"::1"}`.

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::error::AppError;

const VERSION: u8 = 0;
const TYPE_DS: u8 = 0;
const TYPE_NS: u8 = 1;
const TYPE_GLUE4: u8 = 2;
const TYPE_GLUE6: u8 = 3;
const TYPE_SYNTH4: u8 = 4;
const TYPE_SYNTH6: u8 = 5;
const TYPE_TXT: u8 = 6;

fn err(msg: impl Into<String>) -> AppError {
    AppError::InvalidInput(msg.into())
}

fn field<'a>(rec: &'a serde_json::Value, key: &str) -> Result<&'a serde_json::Value, AppError> {
    rec.get(key).ok_or_else(|| err(format!("record missing '{key}'")))
}

fn write_name(out: &mut Vec<u8>, name: &str) -> Result<(), AppError> {
    // Root / empty name is a single 0 byte.
    for label in name.split('.') {
        if label.is_empty() {
            continue; // trailing dot / consecutive dots
        }
        if label.len() > 63 {
            return Err(err(format!("DNS label too long: {label}")));
        }
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    Ok(())
}

fn read_name(buf: &[u8], pos: &mut usize) -> Result<String, AppError> {
    let mut labels = Vec::new();
    loop {
        let len = *buf.get(*pos).ok_or_else(|| err("truncated name"))? as usize;
        *pos += 1;
        if len == 0 {
            break;
        }
        if len > 63 || *pos + len > buf.len() {
            return Err(err("bad DNS label"));
        }
        labels.push(String::from_utf8_lossy(&buf[*pos..*pos + len]).to_string());
        *pos += len;
    }
    Ok(format!("{}.", labels.join(".")))
}

fn write_ipv4(out: &mut Vec<u8>, s: &str) -> Result<(), AppError> {
    let ip: Ipv4Addr = s.parse().map_err(|_| err(format!("bad IPv4: {s}")))?;
    out.extend_from_slice(&ip.octets());
    Ok(())
}

fn write_ipv6(out: &mut Vec<u8>, s: &str) -> Result<(), AppError> {
    let ip: Ipv6Addr = s.parse().map_err(|_| err(format!("bad IPv6: {s}")))?;
    out.extend_from_slice(&ip.octets());
    Ok(())
}

/// Encode a slice of record JSON objects to the raw Resource bytes.
/// An empty slice yields a valid zero-record resource (`[0x00]`).
pub fn encode(records: &[serde_json::Value]) -> Result<Vec<u8>, AppError> {
    let mut out = vec![VERSION];
    for rec in records {
        let kind = field(rec, "type")?.as_str().ok_or_else(|| err("record type must be a string"))?;
        match kind {
            "TXT" => {
                out.push(TYPE_TXT);
                let txt = field(rec, "txt")?.as_array().ok_or_else(|| err("TXT.txt must be an array"))?;
                if txt.len() > 255 {
                    return Err(err("too many TXT strings"));
                }
                out.push(txt.len() as u8);
                for s in txt {
                    let s = s.as_str().ok_or_else(|| err("TXT entry must be a string"))?;
                    if s.len() > 255 {
                        return Err(err("TXT string too long"));
                    }
                    out.push(s.len() as u8);
                    out.extend_from_slice(s.as_bytes());
                }
            }
            "NS" => {
                out.push(TYPE_NS);
                write_name(&mut out, field(rec, "ns")?.as_str().ok_or_else(|| err("NS.ns must be a string"))?)?;
            }
            "GLUE4" | "GLUE6" => {
                out.push(if kind == "GLUE4" { TYPE_GLUE4 } else { TYPE_GLUE6 });
                write_name(&mut out, field(rec, "ns")?.as_str().ok_or_else(|| err("ns must be a string"))?)?;
                let addr = field(rec, "address")?.as_str().ok_or_else(|| err("address must be a string"))?;
                if kind == "GLUE4" { write_ipv4(&mut out, addr)?; } else { write_ipv6(&mut out, addr)?; }
            }
            "SYNTH4" => {
                out.push(TYPE_SYNTH4);
                write_ipv4(&mut out, field(rec, "address")?.as_str().ok_or_else(|| err("address must be a string"))?)?;
            }
            "SYNTH6" => {
                out.push(TYPE_SYNTH6);
                write_ipv6(&mut out, field(rec, "address")?.as_str().ok_or_else(|| err("address must be a string"))?)?;
            }
            "DS" => {
                out.push(TYPE_DS);
                let key_tag = field(rec, "keyTag")?.as_u64().ok_or_else(|| err("DS.keyTag"))? as u16;
                let algorithm = field(rec, "algorithm")?.as_u64().ok_or_else(|| err("DS.algorithm"))? as u8;
                let digest_type = field(rec, "digestType")?.as_u64().ok_or_else(|| err("DS.digestType"))? as u8;
                let digest = hex::decode(
                    field(rec, "digest")?.as_str().ok_or_else(|| err("DS.digest must be hex"))?,
                )
                .map_err(|_| err("DS.digest invalid hex"))?;
                if digest.len() > 255 {
                    return Err(err("DS digest too long"));
                }
                out.extend_from_slice(&key_tag.to_be_bytes());
                out.push(algorithm);
                out.push(digest_type);
                out.push(digest.len() as u8);
                out.extend_from_slice(&digest);
            }
            other => return Err(err(format!("unsupported record type '{other}'"))),
        }
    }
    Ok(out)
}

/// Decode raw Resource bytes to a list of record JSON objects.
pub fn decode(buf: &[u8]) -> Result<Vec<serde_json::Value>, AppError> {
    if buf.is_empty() {
        return Ok(Vec::new()); // EMPTY covenant item = no resource
    }
    if buf[0] != VERSION {
        return Err(err("unsupported resource version"));
    }
    let mut pos = 1usize;
    let mut records = Vec::new();
    while pos < buf.len() {
        let kind = buf[pos];
        pos += 1;
        match kind {
            TYPE_TXT => {
                let count = *buf.get(pos).ok_or_else(|| err("truncated TXT"))? as usize;
                pos += 1;
                let mut txt = Vec::new();
                for _ in 0..count {
                    let len = *buf.get(pos).ok_or_else(|| err("truncated TXT string"))? as usize;
                    pos += 1;
                    if pos + len > buf.len() {
                        return Err(err("truncated TXT bytes"));
                    }
                    txt.push(String::from_utf8_lossy(&buf[pos..pos + len]).to_string());
                    pos += len;
                }
                records.push(serde_json::json!({ "type": "TXT", "txt": txt }));
            }
            TYPE_NS => {
                let ns = read_name(buf, &mut pos)?;
                records.push(serde_json::json!({ "type": "NS", "ns": ns }));
            }
            TYPE_GLUE4 | TYPE_GLUE6 => {
                let ns = read_name(buf, &mut pos)?;
                let (label, n) = if kind == TYPE_GLUE4 { ("GLUE4", 4) } else { ("GLUE6", 16) };
                if pos + n > buf.len() {
                    return Err(err("truncated glue IP"));
                }
                let addr = read_ip(&buf[pos..pos + n]);
                pos += n;
                records.push(serde_json::json!({ "type": label, "ns": ns, "address": addr }));
            }
            TYPE_SYNTH4 | TYPE_SYNTH6 => {
                let (label, n) = if kind == TYPE_SYNTH4 { ("SYNTH4", 4) } else { ("SYNTH6", 16) };
                if pos + n > buf.len() {
                    return Err(err("truncated synth IP"));
                }
                let addr = read_ip(&buf[pos..pos + n]);
                pos += n;
                records.push(serde_json::json!({ "type": label, "address": addr }));
            }
            TYPE_DS => {
                if pos + 5 > buf.len() {
                    return Err(err("truncated DS"));
                }
                let key_tag = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
                let algorithm = buf[pos + 2];
                let digest_type = buf[pos + 3];
                let dlen = buf[pos + 4] as usize;
                pos += 5;
                if pos + dlen > buf.len() {
                    return Err(err("truncated DS digest"));
                }
                let digest = hex::encode(&buf[pos..pos + dlen]);
                pos += dlen;
                records.push(serde_json::json!({
                    "type": "DS", "keyTag": key_tag, "algorithm": algorithm,
                    "digestType": digest_type, "digest": digest
                }));
            }
            other => return Err(err(format!("unknown resource record type {other}"))),
        }
    }
    Ok(records)
}

fn read_ip(bytes: &[u8]) -> String {
    if bytes.len() == 4 {
        Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).to_string()
    } else {
        let mut octets = [0u8; 16];
        octets.copy_from_slice(bytes);
        Ipv6Addr::from(octets).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_resource_round_trips() {
        assert_eq!(encode(&[]).unwrap(), vec![0u8]);
        assert!(decode(&[0u8]).unwrap().is_empty());
        assert!(decode(&[]).unwrap().is_empty());
    }

    #[test]
    fn txt_round_trips() {
        let recs = vec![serde_json::json!({ "type": "TXT", "txt": ["hello", "world"] })];
        let raw = encode(&recs).unwrap();
        // version, type=6, count=2, len5 'hello', len5 'world'
        assert_eq!(raw[0], 0);
        assert_eq!(raw[1], TYPE_TXT);
        assert_eq!(raw[2], 2);
        assert_eq!(decode(&raw).unwrap(), recs);
    }

    #[test]
    fn ns_glue_synth_ds_round_trip() {
        let recs = vec![
            serde_json::json!({ "type": "NS", "ns": "ns1.example." }),
            serde_json::json!({ "type": "GLUE4", "ns": "ns1.example.", "address": "1.2.3.4" }),
            serde_json::json!({ "type": "SYNTH6", "address": "::1" }),
            serde_json::json!({ "type": "DS", "keyTag": 12345, "algorithm": 8, "digestType": 2, "digest": "deadbeef" }),
        ];
        let raw = encode(&recs).unwrap();
        let back = decode(&raw).unwrap();
        assert_eq!(back.len(), 4);
        assert_eq!(back[0]["ns"], "ns1.example.");
        assert_eq!(back[1]["address"], "1.2.3.4");
        assert_eq!(back[2]["address"], "::1");
        assert_eq!(back[3]["keyTag"], 12345);
        assert_eq!(back[3]["digest"], "deadbeef");
    }
}
