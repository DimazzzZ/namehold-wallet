//! HNS ⇄ BTC exchange: cross-chain atomic swaps settled by hash-time-locked
//! contracts, one Commit per party (docs/specs/2026-11-07-hns-btc-exchange.md).
//!
//! Everything here is pure: no node, no network, no database. The IO shells
//! are `commands::exchange` and the daemon's exchange step.

pub mod btc_account;
pub mod btc_htlc;
pub mod btc_spv;
pub mod commit;
pub mod engine;
pub mod fees;
pub mod hns_htlc;
pub mod keys;
pub mod policy;
pub mod price;
pub mod relay_msg;
pub mod review;
pub mod script;
pub mod terms;
pub mod watch;
