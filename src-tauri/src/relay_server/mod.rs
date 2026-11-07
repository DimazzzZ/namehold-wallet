//! The relay and watchtower server of the HNS ⇄ BTC exchange (spec §8,
//! docs/RELAY.md), without its HTTP binding (`src/bin/namehold-relay.rs`,
//! feature `relay-server`), so everything here is built and tested with the
//! default features.
//!
//! The server holds no key and no secret before it is public. The relay
//! stores only what it verified; the watchtower sends only what a party
//! signed. At worst it hides offers or messages, or fails to send.

pub mod config;
pub mod handlers;
pub mod store;
pub mod watchtower;
