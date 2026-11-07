//! The IO side of the HNS ⇄ BTC exchange: reading the two chains, the
//! relay, and the database. The decisions are in `noncustodial::exchange`.

pub mod btc_observe;
pub mod btc_source;
pub mod hns_observe;
pub mod relay_client;
pub mod runner;
