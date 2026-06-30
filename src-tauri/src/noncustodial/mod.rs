//! Non-custodial signing engine for Handshake.
//!
//! This module holds all logic for a wallet model where Namehold holds the
//! keys and signs transactions locally, using a node only for broadcast and
//! chain reads. All Handshake-specific constants are verified against the
//! canonical `hsd` source (see per-module doc comments).

pub mod address;
pub mod derivation;
pub mod hd;
pub mod network;
pub mod rpc;
pub mod send;
pub mod session;
pub mod sync;
pub mod tx;
pub mod vault;
