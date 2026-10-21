//! Finding our own Shakedex locks after a wallet restore (R34): the candidate
//! filter over the transactions the node reports for our addresses. A lock
//! starts as a TRANSFER at one of our addresses that commits the name to an
//! address that is not ours; whether that address is a lock of our key is
//! decided by [`commits_to_lock`] once the lock key is derived.

use std::collections::HashSet;

use crate::noncustodial::address;
use crate::noncustodial::network::Network;
use crate::noncustodial::sync::COV_TRANSFER;

/// A TRANSFER at one of our addresses that hands a name to some other address:
/// the shape every lock starts with, and also every ordinary transfer to
/// someone else.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LockCandidate {
    /// The name's hash, 64 hex characters.
    pub name_hash: String,
    /// The address the TRANSFER commits the name to.
    pub committed: String,
}

/// The address a TRANSFER's commitment (items 2-3: address version and hash)
/// names: version 0 with a 20-byte (P2WPKH) or 32-byte (P2WSH) hash; `None`
/// for anything else, including items that are missing or not hex.
fn committed_address(items: &[&str], network: Network) -> Option<String> {
    let [_, _, version, hash, ..] = items else {
        return None;
    };
    if hex::decode(version).ok()?.as_slice() != [0] {
        return None;
    }
    let hash = hex::decode(hash).ok()?;
    if let Ok(h) = <[u8; 20]>::try_from(hash.as_slice()) {
        address::encode_p2wpkh(network, &h).ok()
    } else if let Ok(h) = <[u8; 32]>::try_from(hash.as_slice()) {
        address::encode_p2wsh(network, &h).ok()
    } else {
        None
    }
}

/// The lock candidates in `txs` (hsd's transaction JSON, as `GET /tx/address`
/// returns it): every output at an address in `own` with a TRANSFER covenant
/// that commits to an address not in `own`. A malformed output is skipped, not
/// an error: the same transaction arrives once per address it touches, so a
/// candidate is listed once, in first-seen order.
pub fn lock_candidates(
    txs: &[serde_json::Value],
    own: &HashSet<String>,
    network: Network,
) -> Vec<LockCandidate> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for output in txs
        .iter()
        .filter_map(|t| t.get("outputs").and_then(|o| o.as_array()))
        .flatten()
    {
        let at_ours = output
            .get("address")
            .and_then(|a| a.as_str())
            .is_some_and(|a| own.contains(a));
        let Some(cov) = output.get("covenant").filter(|_| at_ours) else {
            continue;
        };
        if cov.get("type").and_then(|t| t.as_u64()) != Some(u64::from(COV_TRANSFER)) {
            continue;
        }
        let Some(items) = cov
            .get("items")
            .and_then(|i| i.as_array())
            .and_then(|i| i.iter().map(|s| s.as_str()).collect::<Option<Vec<_>>>())
        else {
            continue;
        };
        let name_hash = match items.first() {
            Some(h) if h.len() == 64 && hex::decode(h).is_ok() => h.to_ascii_lowercase(),
            _ => continue,
        };
        let Some(committed) = committed_address(&items, network) else {
            continue;
        };
        if own.contains(&committed) {
            continue;
        }
        let candidate = LockCandidate {
            name_hash,
            committed,
        };
        if seen.insert(candidate.clone()) {
            out.push(candidate);
        }
    }
    out
}

/// Whether the candidate commits to the lock whose witness program is
/// `lock_program` (the lock key's script hash).
pub fn commits_to_lock(c: &LockCandidate, lock_program: &[u8; 32], network: Network) -> bool {
    address::encode_p2wsh(network, lock_program).is_ok_and(|a| a == c.committed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NET: Network = Network::Regtest;
    const NAME: &str = "aa00000000000000000000000000000000000000000000000000000000000011";

    fn ours() -> (String, HashSet<String>) {
        let a = address::encode_p2wpkh(NET, &[1; 20]).unwrap();
        (a.clone(), HashSet::from([a]))
    }

    fn output(at: &str, cov_type: u8, items: serde_json::Value) -> serde_json::Value {
        json!({"address": at, "covenant": {"type": cov_type, "action": "TRANSFER", "items": items}})
    }

    fn transfer(at: &str, version: &str, hash_hex: &str) -> serde_json::Value {
        output(at, 9, json!([NAME, "01000000", version, hash_hex]))
    }

    fn tx(outputs: Vec<serde_json::Value>) -> serde_json::Value {
        json!({"outputs": outputs})
    }

    #[test]
    fn transfer_at_our_address_to_another_is_a_candidate() {
        let (me, own) = ours();
        let program = [7u8; 32];
        let got = lock_candidates(
            &[tx(vec![transfer(&me, "00", &hex::encode(program))])],
            &own,
            NET,
        );
        assert_eq!(
            got,
            vec![LockCandidate {
                name_hash: NAME.into(),
                committed: address::encode_p2wsh(NET, &program).unwrap(),
            }]
        );
        let p2wpkh = lock_candidates(
            &[tx(vec![transfer(&me, "00", &"02".repeat(20))])],
            &own,
            NET,
        );
        assert_eq!(p2wpkh.len(), 1);
    }

    #[test]
    fn transfer_to_one_of_our_addresses_is_not() {
        let (me, own) = ours();
        let got = lock_candidates(
            &[tx(vec![transfer(&me, "00", &"01".repeat(20))])],
            &own,
            NET,
        );
        assert!(got.is_empty());
    }

    #[test]
    fn outputs_not_ours_or_not_transfer_are_ignored() {
        let (me, own) = ours();
        let foreign = address::encode_p2wpkh(NET, &[9; 20]).unwrap();
        let finalize = output(&me, 10, json!([NAME, "01000000", "00", "07".repeat(32)]));
        let got = lock_candidates(
            &[tx(vec![
                finalize,
                transfer(&foreign, "00", &"07".repeat(32)),
            ])],
            &own,
            NET,
        );
        assert!(got.is_empty());
    }

    #[test]
    fn the_same_candidate_twice_is_listed_once() {
        let (me, own) = ours();
        let t = tx(vec![transfer(&me, "00", &"07".repeat(32))]);
        let got = lock_candidates(&[t.clone(), t], &own, NET);
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn a_malformed_covenant_is_skipped() {
        let (me, own) = ours();
        let bad = vec![
            json!({"address": me, "covenant": {"type": 9}}),
            json!({"address": me}),
            transfer(&me, "00", "zz"),
            transfer(&me, "01", &"07".repeat(32)),
            transfer(&me, "00", &"07".repeat(31)),
            output(&me, 9, json!([NAME])),
            output(
                &me,
                9,
                json!(["not-a-hash", "01000000", "00", "07".repeat(32)]),
            ),
            output(&me, 9, json!([NAME, "01000000", 0, 1])),
        ];
        assert!(lock_candidates(&[tx(bad)], &own, NET).is_empty());
        assert!(lock_candidates(&[json!({}), json!(null)], &own, NET).is_empty());
    }

    #[test]
    fn commits_to_lock_compares_the_program() {
        let c = LockCandidate {
            name_hash: NAME.into(),
            committed: address::encode_p2wsh(NET, &[7; 32]).unwrap(),
        };
        assert!(commits_to_lock(&c, &[7; 32], NET));
        assert!(!commits_to_lock(&c, &[8; 32], NET));
    }
}
