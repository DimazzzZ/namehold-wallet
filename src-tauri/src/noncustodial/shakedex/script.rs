//! The Shakedex lock: `OP_TYPE TRANSFER EQUAL IF <pub> CHECKSIG ELSE
//! OP_TYPE FINALIZE EQUAL ENDIF`, paid to as P2WSH (program = SHA3-256 of
//! the script). TRANSFER out needs the lock key; FINALIZE out needs nothing.

use sha3::{Digest, Sha3_256};

use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::network::Network;

const PREFIX: [u8; 4] = [0xd0, 0x59, 0x87, 0x63]; // OP_TYPE OP_9 OP_EQUAL OP_IF
const SUFFIX: [u8; 6] = [0xac, 0x67, 0xd0, 0x5a, 0x87, 0x68]; // CHECKSIG ELSE TYPE OP_10 EQUAL ENDIF

pub fn lock_script(pubkey: &[u8; 33]) -> Vec<u8> {
    let mut s = Vec::with_capacity(44);
    s.extend_from_slice(&PREFIX);
    s.push(0x21);
    s.extend_from_slice(pubkey);
    s.extend_from_slice(&SUFFIX);
    s
}

pub fn lock_program(pubkey: &[u8; 33]) -> [u8; 32] {
    Sha3_256::digest(lock_script(pubkey)).into()
}

pub fn lock_address(network: Network, pubkey: &[u8; 33]) -> Result<String, AppError> {
    address::encode_p2wsh(network, &lock_program(pubkey))
}

pub fn is_lock_script_for(script: &[u8], pubkey: &[u8; 33]) -> bool {
    script == lock_script(pubkey).as_slice()
}
