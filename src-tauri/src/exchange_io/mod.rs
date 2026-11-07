//! The IO side of the HNS ⇄ BTC exchange: reading the two chains, the
//! relay, and the database. The decisions are in `noncustodial::exchange`.

pub mod hns_observe;
