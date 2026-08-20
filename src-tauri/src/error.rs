use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Lock error: {0}")]
    Lock(String),
    #[error("Crypto error: {0}")]
    Crypto(String),
    #[error("Node RPC error: {0}")]
    Rpc(String),
    #[error("Wallet locked")]
    WalletLocked,
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    /// The user declined a secure-window confirmation (e.g. cancelled the
    /// per-transaction spend confirmation). Kept distinct from `Other` so the
    /// frontend can treat a deliberate cancel as a benign, non-error outcome
    /// rather than a failure to surface loudly.
    #[error("Confirmation declined")]
    UserRejected,
    /// A Namebase response had a successful HTTP status but its body looks like
    /// the HTML login page rather than API JSON — Namebase's way of soft-expiring
    /// a session without a 401. Kept distinct from `Other` so the client layer
    /// can raise this instead of an ugly JSON-parse error.
    #[error("Namebase session expired — reconnect with a fresh cookie")]
    NamebaseSessionExpired,
    /// The Namebase API returned 429 Too Many Requests. `retry_after_secs` is
    /// parsed from the `Retry-After` response header when present, otherwise a
    /// sensible default. Kept distinct from `Other` so the frontend can render
    /// an actionable "try again in Ns" message rather than a raw status string.
    /// This mainly affects the heavy `/api/account/history/export` endpoint,
    /// which Namebase rate-limits more strictly than the lightweight list APIs.
    #[error("Namebase rate limit exceeded (retry after {retry_after_secs}s)")]
    NamebaseRateLimited { retry_after_secs: u64 },
    /// An explorer answered with a successful HTTP status, but its response
    /// body doesn't match the shape the client expects (e.g. a renamed/removed
    /// field). Kept distinct from `Other`/transport errors so callers can tell
    /// "the explorer's contract drifted" apart from "the name genuinely has no
    /// data" or "the explorer is unreachable" — conflating these used to make
    /// a format change degrade silently into "you own nothing" (Task 11 / S1).
    #[error("Explorer response format unrecognized: {0}")]
    ExplorerFormat(String),
    /// A hardware wallet (Ledger) transport, protocol, or on-device error.
    /// Kept distinct from `Other`/`Crypto` so the frontend can render
    /// actionable device guidance (unplug/reconnect, unlock, open the HNS app,
    /// approve the on-screen prompt) rather than a generic failure. The string
    /// carries the specific cause (e.g. "device not found", "HNS app not open",
    /// APDU status word `0x6985` = user rejected).
    ///
    /// Reserve `Device` for genuine transport / device-state / on-device
    /// causes. Programmer errors detected pre-flight (oversized transactions,
    /// bad hex we produced ourselves, missing pre-resolved names) belong in
    /// [`AppError::Protocol`] — see the note there.
    #[error("Ledger device error: {0}")]
    Device(String),
    /// A hardware-wallet protocol / wire-format precondition failure detected
    /// *before* any bytes are sent to the device. These are effectively
    /// programmer errors on the client side (oversized inputs/outputs counts,
    /// bad hex we ourselves produced, missing pre-resolved covenant names,
    /// out-of-range varints). Kept distinct from `Device` so:
    ///   1. The frontend can surface "internal / please report" rather than
    ///      the actionable "unplug and reconnect" guidance `Device` implies.
    ///   2. Callers and tests can pattern-match programmer bugs without
    ///      false-positive matches on transport hiccups.
    #[error("Ledger protocol error: {0}")]
    Protocol(String),
    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
