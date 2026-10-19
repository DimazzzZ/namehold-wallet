//! Single entry point for building `reqwest` clients.
//!
//! reqwest is built with `rustls-no-provider`, so a rustls crypto provider has
//! to be installed before the first client is built — otherwise reqwest
//! panics. We install `ring`, the same provider tauri-plugin-updater installs
//! for its own requests, so the binary carries one TLS crypto backend instead
//! of `ring` plus reqwest's default `aws-lc-rs`.

use std::sync::Once;

static INSTALL_PROVIDER: Once = Once::new();

/// `reqwest::Client::builder()` with the process-wide rustls provider in place.
pub(crate) fn client_builder() -> reqwest::ClientBuilder {
    INSTALL_PROVIDER.call_once(|| {
        // Err means another provider is already installed (e.g. by the
        // updater plugin), which is just as good.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::Client::builder()
}

#[cfg(test)]
mod tests {
    #[test]
    fn builds_an_https_capable_client() {
        super::client_builder().build().expect("client builds");
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
