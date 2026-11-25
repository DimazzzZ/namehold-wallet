//! Single entry point for building `reqwest` clients.
//!
//! reqwest is built with `rustls-no-provider`, so a rustls crypto provider has
//! to be installed before the first client is built — otherwise reqwest
//! panics. We install `ring`, the same provider tauri-plugin-updater installs
//! for its own requests, so the binary carries one TLS crypto backend instead
//! of `ring` plus reqwest's default `aws-lc-rs`.

use std::sync::Once;

static INSTALL_PROVIDER: Once = Once::new();

fn base_builder() -> reqwest::ClientBuilder {
    INSTALL_PROVIDER.call_once(|| {
        // Err means another provider is already installed (e.g. by the
        // updater plugin), which is just as good.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    reqwest::Client::builder().connect_timeout(CONNECT_TIMEOUT)
}

/// How long one connection may take to open, across all the addresses its
/// host resolves to: hyper splits it evenly between them, so an address that
/// drops the handshake is left after a share of it for the next one. Without
/// it the request's own timeout ran out on the first such address
/// (mempool.space resolves to 16, and from one home network ~60% of them
/// hung for ~15 s: 14 of 30 fresh requests failed at the 20 s timeout, none
/// with this bound; QA 2026-11-25).
pub(crate) const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// `e` with its causes: reqwest's own text stops at "error sending request
/// for url (...)" and keeps why (a timeout, a refused connection, a TLS
/// failure) in its source chain.
pub(crate) fn describe(e: &reqwest::Error) -> String {
    let mut s = e.to_string();
    let mut cause = std::error::Error::source(e);
    while let Some(c) = cause {
        let c_text = c.to_string();
        if !s.contains(&c_text) {
            s.push_str(": ");
            s.push_str(&c_text);
        }
        cause = c.source();
    }
    s
}

/// `reqwest::Client::builder()` with the process-wide rustls provider in place.
pub(crate) fn client_builder() -> reqwest::ClientBuilder {
    let builder = base_builder();
    // Tests reach only local mock servers. Reading the system proxy on macOS
    // (`SCDynamicStoreCreate`) lists the directory the executable sits in,
    // which for a test binary is cargo's `deps/`; nextest pays that once per
    // test process.
    #[cfg(test)]
    let builder = builder.no_proxy();
    builder
}

/// Whether `url`'s host is this machine: `localhost`, `127.0.0.0/8` or `::1`.
pub(crate) fn is_loopback_url(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(a)) => a.is_loopback(),
        Some(url::Host::Ipv6(a)) => a.is_loopback(),
        None => false,
    }
}

/// The builder of a client for a chain source at `url` (the node, the
/// explorer, and the exchange's relay, watchtower and Bitcoin sources), none
/// of which is trusted to send it elsewhere:
///
/// - No redirect is followed. Neither hsd nor the explorer has a reason to
///   send one; a 3xx comes back as an answer the caller refuses.
/// - A loopback URL ignores the system and environment proxy: reqwest has no
///   loopback exemption of its own, so `http://127.0.0.1` would go through
///   `HTTP_PROXY` (or macOS's web proxy) with the api-key in a cleartext
///   header, the one case [`crate::noncustodial::rpc::guard_transport`]
///   exists to refuse.
/// - `localhost` connects to `127.0.0.1` whatever the resolver says, so where
///   the api-key goes is not left to it. hsd listens on `127.0.0.1` by
///   default; a node on `::1` only is reached as `http://[::1]:<port>`.
pub(crate) fn chain_source_client_builder(url: &str) -> reqwest::ClientBuilder {
    let loopback = url::Url::parse(url.trim()).is_ok_and(|u| is_loopback_url(&u));
    let builder = if loopback {
        base_builder().no_proxy().resolve(
            "localhost",
            // The port is ignored: the URL's own is used.
            std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0)),
        )
    } else {
        client_builder()
    };
    builder.redirect(reqwest::redirect::Policy::none())
}

/// Why [`read_capped`] returned no body.
#[derive(Debug)]
pub(crate) enum BodyError {
    TooLarge(usize),
    Transport(reqwest::Error),
}

impl std::fmt::Display for BodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BodyError::TooLarge(max) => write!(f, "response is larger than {max} bytes"),
            BodyError::Transport(e) => e.fmt(f),
        }
    }
}

/// Read `resp`'s body; one over `max_bytes` is refused, and at most
/// `max_bytes` plus one chunk is ever held.
pub(crate) async fn read_capped(
    mut resp: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, BodyError> {
    if resp.content_length().is_some_and(|n| n > max_bytes as u64) {
        return Err(BodyError::TooLarge(max_bytes));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(BodyError::Transport)? {
        if body.len() + chunk.len() > max_bytes {
            return Err(BodyError::TooLarge(max_bytes));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    #[test]
    fn builds_an_https_capable_client() {
        super::client_builder().build().expect("client builds");
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
