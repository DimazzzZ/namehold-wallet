//! Covenant builders for Handshake name actions.
//!
//! Each builder returns a [`tx::Covenant`] whose `items` are in the exact order
//! hsd v6.1.1 `lib/wallet/wallet.js` pushes them. Covenant pushes map to items
//! as: `pushHash(h)` → raw bytes; `pushU32(n)` → 4-byte LE; `pushU8(n)` → 1
//! byte; `push(buf)` → buf (an empty item for `EMPTY`).
//!
//! Name covenants live on transaction OUTPUTS. The builders take the already
//! -computed values (height, blind, nonce, resource, renewal block, …) from the
//! caller (which has node/name-state access); they are pure and unit-testable.

use crate::noncustodial::sync::{
    COV_BID, COV_FINALIZE, COV_OPEN, COV_REDEEM, COV_REGISTER, COV_RENEW, COV_REVEAL, COV_REVOKE,
    COV_TRANSFER, COV_UPDATE,
};
use crate::noncustodial::tx::Covenant;

fn u32le(n: u32) -> Vec<u8> {
    n.to_le_bytes().to_vec()
}

/// OPEN: value 0. items `[nameHash, u32(0), rawName]`.
pub fn open(name_hash: &[u8; 32], raw_name: &[u8]) -> Covenant {
    Covenant {
        covenant_type: COV_OPEN,
        items: vec![name_hash.to_vec(), u32le(0), raw_name.to_vec()],
    }
}

/// BID: value = lockup. items `[nameHash, u32(start), rawName, blind]`.
/// `start` is the OPEN height of the auction (name-state `height`).
pub fn bid(name_hash: &[u8; 32], start: u32, raw_name: &[u8], blind: &[u8; 32]) -> Covenant {
    Covenant {
        covenant_type: COV_BID,
        items: vec![
            name_hash.to_vec(),
            u32le(start),
            raw_name.to_vec(),
            blind.to_vec(),
        ],
    }
}

/// REVEAL: value = true bid value. items `[nameHash, u32(height), nonce]`.
pub fn reveal(name_hash: &[u8; 32], height: u32, nonce: &[u8; 32]) -> Covenant {
    Covenant {
        covenant_type: COV_REVEAL,
        items: vec![name_hash.to_vec(), u32le(height), nonce.to_vec()],
    }
}

/// REDEEM (reclaim a losing bid's lockup). items `[nameHash, u32(height)]`.
pub fn redeem(name_hash: &[u8; 32], height: u32) -> Covenant {
    Covenant {
        covenant_type: COV_REDEEM,
        items: vec![name_hash.to_vec(), u32le(height)],
    }
}

/// REGISTER: items `[nameHash, u32(height), resource | EMPTY, renewalBlock]`.
pub fn register(
    name_hash: &[u8; 32],
    height: u32,
    resource: &[u8],
    renewal_block: &[u8; 32],
) -> Covenant {
    Covenant {
        covenant_type: COV_REGISTER,
        items: vec![
            name_hash.to_vec(),
            u32le(height),
            resource.to_vec(),
            renewal_block.to_vec(),
        ],
    }
}

/// UPDATE: items `[nameHash, u32(height), resource]`.
pub fn update(name_hash: &[u8; 32], height: u32, resource: &[u8]) -> Covenant {
    Covenant {
        covenant_type: COV_UPDATE,
        items: vec![name_hash.to_vec(), u32le(height), resource.to_vec()],
    }
}

/// RENEW: items `[nameHash, u32(height), renewalBlock]`.
pub fn renew(name_hash: &[u8; 32], height: u32, renewal_block: &[u8; 32]) -> Covenant {
    Covenant {
        covenant_type: COV_RENEW,
        items: vec![name_hash.to_vec(), u32le(height), renewal_block.to_vec()],
    }
}

/// TRANSFER: items `[nameHash, u32(height), u8(addrVersion), addrHash]`.
pub fn transfer(name_hash: &[u8; 32], height: u32, addr_version: u8, addr_hash: &[u8]) -> Covenant {
    Covenant {
        covenant_type: COV_TRANSFER,
        items: vec![
            name_hash.to_vec(),
            u32le(height),
            vec![addr_version],
            addr_hash.to_vec(),
        ],
    }
}

/// FINALIZE: items `[nameHash, u32(height), rawName, u8(flags), u32(claimed),
/// u32(renewals), renewalBlock]`.
#[allow(clippy::too_many_arguments)]
pub fn finalize(
    name_hash: &[u8; 32],
    height: u32,
    raw_name: &[u8],
    flags: u8,
    claimed: u32,
    renewals: u32,
    renewal_block: &[u8; 32],
) -> Covenant {
    Covenant {
        covenant_type: COV_FINALIZE,
        items: vec![
            name_hash.to_vec(),
            u32le(height),
            raw_name.to_vec(),
            vec![flags],
            u32le(claimed),
            u32le(renewals),
            renewal_block.to_vec(),
        ],
    }
}

/// CANCEL (revert an in-flight TRANSFER). NOTE: hsd encodes this as an UPDATE
/// covenant with an EMPTY resource: items `[nameHash, u32(height), EMPTY]`.
pub fn cancel(name_hash: &[u8; 32], height: u32) -> Covenant {
    Covenant {
        covenant_type: COV_UPDATE,
        items: vec![name_hash.to_vec(), u32le(height), Vec::new()],
    }
}

/// REVOKE: items `[nameHash, u32(height)]`.
pub fn revoke(name_hash: &[u8; 32], height: u32) -> Covenant {
    Covenant {
        covenant_type: COV_REVOKE,
        items: vec![name_hash.to_vec(), u32le(height)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_layout() {
        let nh = [9u8; 32];
        let cov = open(&nh, b"example");
        assert_eq!(cov.covenant_type, COV_OPEN);
        assert_eq!(cov.items.len(), 3);
        assert_eq!(cov.items[0], nh.to_vec());
        assert_eq!(cov.items[1], vec![0, 0, 0, 0]); // u32 LE 0
        assert_eq!(cov.items[2], b"example".to_vec());
    }

    #[test]
    fn bid_and_reveal_layout() {
        let nh = [1u8; 32];
        let blind = [2u8; 32];
        let nonce = [3u8; 32];
        let b = bid(&nh, 100, b"abc", &blind);
        assert_eq!(b.covenant_type, COV_BID);
        assert_eq!(b.items.len(), 4);
        assert_eq!(b.items[1], 100u32.to_le_bytes().to_vec());
        assert_eq!(b.items[3], blind.to_vec());

        let r = reveal(&nh, 150, &nonce);
        assert_eq!(r.covenant_type, COV_REVEAL);
        assert_eq!(r.items, vec![nh.to_vec(), 150u32.to_le_bytes().to_vec(), nonce.to_vec()]);
    }

    #[test]
    fn transfer_and_finalize_and_cancel_layout() {
        let nh = [4u8; 32];
        let rb = [5u8; 32];
        let t = transfer(&nh, 200, 0, &[6u8; 20]);
        assert_eq!(t.covenant_type, COV_TRANSFER);
        assert_eq!(t.items[2], vec![0u8]); // version byte
        assert_eq!(t.items[3], vec![6u8; 20]);

        let f = finalize(&nh, 200, b"abc", 0, 0, 3, &rb);
        assert_eq!(f.covenant_type, COV_FINALIZE);
        assert_eq!(f.items.len(), 7);
        assert_eq!(f.items[5], 3u32.to_le_bytes().to_vec()); // renewals

        // CANCEL is an UPDATE covenant with an empty resource item.
        let c = cancel(&nh, 200);
        assert_eq!(c.covenant_type, COV_UPDATE);
        assert_eq!(c.items.len(), 3);
        assert!(c.items[2].is_empty());
    }
}
