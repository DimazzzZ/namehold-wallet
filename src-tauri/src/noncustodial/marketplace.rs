//! Canonical marketplace and swap protocol boundary.
//!
//! Namehold never defines marketplace wire objects locally. Callers decode and
//! verify the pinned `hns-rs` types before persisting or displaying them, while
//! tracked-chain evidence comes from authenticated wallet RPC v1.

pub use hns_marketplace_protocol::*;
pub use hns_swap::{
    FixedPriceListing, HnsHtlc, HnsHtlcPreimage, HnsHtlcSpend, ListingCancellation, NetworkBinding,
    SwapProof,
};

use crate::error::AppError;

/// Decode and fully verify one fixed-price listing envelope.
pub fn decode_fixed_price_listing(
    encoded: &[u8],
) -> std::result::Result<FixedPriceListing, AppError> {
    FixedPriceListing::decode(encoded)
        .map_err(|error| AppError::InvalidInput(format!("invalid marketplace listing: {error}")))
}

/// Decode and verify one signed cancellation envelope.
pub fn decode_listing_cancellation(
    encoded: &[u8],
) -> std::result::Result<ListingCancellation, AppError> {
    ListingCancellation::decode(encoded).map_err(|error| {
        AppError::InvalidInput(format!("invalid marketplace cancellation: {error}"))
    })
}
