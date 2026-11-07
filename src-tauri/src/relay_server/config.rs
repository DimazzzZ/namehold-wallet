//! The server's configuration (docs/RELAY.md): where it listens, its
//! database, and per served network the HNS node and the Bitcoin sources
//! of the watchtower.
//!
//! A network without a node is served by the relay only: the server then
//! refuses watch jobs of that network rather than accept one it cannot run.
//! Everything is checked when the file is read, so a server never starts
//! with a node URL that would send its API key in the clear or a watchtower
//! without a Bitcoin source.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;

use serde::Deserialize;

use crate::error::AppError;
use crate::exchange_io::btc_source;
use crate::noncustodial::network::Network;

/// The file as written (JSON).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfigFile {
    listen: String,
    db_path: String,
    networks: BTreeMap<String, NetworkFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NetworkFile {
    #[serde(default)]
    node: Option<NodeFile>,
    /// Absent: `btc_source::default_sources` of the network.
    #[serde(default)]
    btc_sources: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NodeFile {
    url: String,
    #[serde(default)]
    api_key: String,
}

impl std::fmt::Debug for NodeFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeFile")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

/// The hsd node of a network's watchtower.
#[derive(Clone)]
pub struct NodeConfig {
    pub url: String,
    pub api_key: String,
}

// The API key is a bearer credential: never in a log line.
impl std::fmt::Debug for NodeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeConfig")
            .field("url", &self.url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone)]
pub struct NetworkConfig {
    /// `None`: the watchtower is off for this network; the relay serves it.
    pub node: Option<NodeConfig>,
    /// Source specs (`btc_source::source_from_spec`).
    pub btc_sources: Vec<String>,
}

// A `bitcoind+http://user:pass@…` spec carries a password: count only.
impl std::fmt::Debug for NetworkConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkConfig")
            .field("node", &self.node)
            .field("btc_sources", &self.btc_sources.len())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub db_path: PathBuf,
    pub networks: HashMap<Network, NetworkConfig>,
}

fn bad(m: impl std::fmt::Display) -> AppError {
    AppError::InvalidInput(format!("relay config: {m}"))
}

/// `https://` anywhere, `http://` only to this machine.
fn check_node_url(network: Network, url: &str) -> Result<(), AppError> {
    let parsed = url::Url::parse(url.trim()).map_err(|e| {
        bad(format!(
            "{}: the node URL is not a URL: {e}",
            network.as_str()
        ))
    })?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if crate::http::is_loopback_url(&parsed) => Ok(()),
        _ => Err(bad(format!(
            "{}: the node must be https, or http to this machine: {url}",
            network.as_str()
        ))),
    }
}

impl Config {
    pub fn parse(json: &str) -> Result<Config, AppError> {
        let f: ConfigFile = serde_json::from_str(json).map_err(bad)?;
        let listen: SocketAddr = f
            .listen
            .parse()
            .map_err(|e| bad(format!("listen '{}': {e}", f.listen)))?;
        if f.db_path.trim().is_empty() {
            return Err(bad("dbPath is empty"));
        }
        if f.networks.is_empty() {
            return Err(bad("no network is served"));
        }
        let mut networks = HashMap::new();
        for (name, n) in f.networks {
            // The canonical spelling only: offers are matched by it.
            let Some(network) = Network::from_str_opt(&name).filter(|n| n.as_str() == name) else {
                return Err(bad(format!(
                    "'{name}' is not a network (main, testnet, regtest, simnet)"
                )));
            };
            let node = match n.node {
                Some(node) => {
                    check_node_url(network, &node.url)?;
                    crate::noncustodial::rpc::guard_transport(&node.url, &node.api_key)?;
                    Some(NodeConfig {
                        url: node.url,
                        api_key: node.api_key,
                    })
                }
                None => None,
            };
            let btc_sources = n
                .btc_sources
                .unwrap_or_else(|| btc_source::default_sources(network));
            for s in &btc_sources {
                btc_source::source_from_spec(s)
                    .map_err(|e| bad(format!("{}: {e}", network.as_str())))?;
            }
            if node.is_some() && btc_sources.is_empty() {
                return Err(bad(format!(
                    "{}: the watchtower needs at least one Bitcoin source (btcSources)",
                    network.as_str()
                )));
            }
            networks.insert(network, NetworkConfig { node, btc_sources });
        }
        Ok(Config {
            listen,
            db_path: PathBuf::from(f.db_path),
            networks,
        })
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn load(path: &std::path::Path) -> Result<Config, AppError> {
        let json = std::fs::read_to_string(path)
            .map_err(|e| bad(format!("cannot read {}: {e}", path.display())))?;
        Config::parse(&json)
    }

    /// The served network spelled `name` (canonical spelling only).
    pub fn serves(&self, name: &str) -> Option<Network> {
        Network::from_str_opt(name)
            .filter(|n| n.as_str() == name)
            .filter(|n| self.networks.contains_key(n))
    }

    /// Whether the watchtower runs for `n` (the network has a node).
    pub fn watches(&self, n: Network) -> bool {
        self.networks.get(&n).is_some_and(|c| c.node.is_some())
    }

    /// The served networks, as `GET /v1/health` lists them.
    pub fn network_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.networks.keys().map(|n| n.as_str()).collect();
        names.sort_unstable();
        names
    }
}
