//! Cancelling a listing (R28): the lock key signs the lock coin
//! `ANYONECANPAY|SINGLE` into a TRANSFER at the lock address committing to
//! a reserved address of ours, and after the lockup a FINALIZE out of the
//! lock brings the name there.

use crate::noncustodial::tx::sighash;

/// `ANYONECANPAY|SINGLE`: the lock key commits to the lock input and the
/// TRANSFER at the same index only, as shakedex's cancel does.
pub const CANCEL_SIGHASH: u32 = sighash::ANYONECANPAY | sighash::SINGLE;
