//! `namehold-relay` — the relay and watchtower server of the HNS ⇄ BTC
//! exchange (spec §8, docs/RELAY.md). Built only with the `relay-server`
//! feature.
//!
//! This file is the HTTP binding only: every request goes to
//! `relay_server::handlers::dispatch`, the watchtower round is
//! `relay_server::watchtower::tick` (every 60 s) and the purge
//! `relay_server::handlers::purge` (every hour). It holds no key and no
//! secret; one log line per request names the method, the path and the
//! status, never a body or a query.
//!
//! Usage: `namehold-relay <config.json>`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Router;

use namehold_wallet_lib::exchange_io::btc_source::{source_from_spec, BtcSource};
use namehold_wallet_lib::noncustodial::exchange::relay_msg::MAX_MESSAGE_BYTES;
use namehold_wallet_lib::noncustodial::network::Network;
use namehold_wallet_lib::noncustodial::node_rpc::NodeRpc;
use namehold_wallet_lib::noncustodial::rpc::{ChainSource, NodeRpcClient};
use namehold_wallet_lib::relay_server::config::Config;
use namehold_wallet_lib::relay_server::handlers::{self, Reply};
use namehold_wallet_lib::relay_server::store::Store;
use namehold_wallet_lib::relay_server::watchtower::{self, NetworkIo};

const TICK_EVERY: Duration = Duration::from_secs(60);
const PURGE_EVERY: Duration = Duration::from_secs(3600);

fn main() {
    namehold_wallet_lib::harden_process();
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: namehold-relay <config.json>   (docs/RELAY.md)");
        std::process::exit(2);
    };
    let cfg = match Config::load(std::path::Path::new(&path)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("namehold-relay: {e}");
            std::process::exit(2);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("namehold-relay: could not start the runtime: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = rt.block_on(run(cfg)) {
        eprintln!("namehold-relay: {e}");
        std::process::exit(1);
    }
}

fn now() -> u64 {
    // A clock before 1970 reads as 0: every offer and job then looks
    // unexpired-but-early, and no refund time is ever reached.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn log(line: impl std::fmt::Display) {
    eprintln!("{} {line}", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"));
}

struct App {
    store: Arc<Store>,
    cfg: Config,
}

/// One network's watchtower IO, owned by the watchtower task.
struct OwnedIo {
    network: Network,
    node: NodeRpcClient,
    sources: Vec<Box<dyn BtcSource>>,
}

async fn run(cfg: Config) -> Result<(), String> {
    let store = Arc::new(
        Store::open(&cfg.db_path)
            .map_err(|e| format!("cannot open {}: {e}", cfg.db_path.display()))?,
    );

    let mut io = Vec::new();
    for (network, n) in &cfg.networks {
        let Some(node) = &n.node else {
            log(format!(
                "{}: relay only (no node: the watchtower is off)",
                network.as_str()
            ));
            continue;
        };
        let client = NodeRpcClient::try_new(&node.url, &node.api_key, ChainSource::LocalNode)
            .map_err(|e| format!("{}: {e}", network.as_str()))?;
        let sources = n
            .btc_sources
            .iter()
            .map(|s| source_from_spec(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{}: {e}", network.as_str()))?;
        log(format!(
            "{}: relay and watchtower (node {}, {} Bitcoin source(s))",
            network.as_str(),
            node.url,
            sources.len()
        ));
        io.push(OwnedIo {
            network: *network,
            node: client,
            sources,
        });
    }

    let watch_store = store.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(TICK_EVERY);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            let map: HashMap<Network, NetworkIo<'_>> = io
                .iter()
                .map(|o| {
                    (
                        o.network,
                        NetworkIo {
                            node: Some(&o.node as &dyn NodeRpc),
                            sources: &o.sources,
                        },
                    )
                })
                .collect();
            match watchtower::tick(&watch_store, &map, now()).await {
                Ok(outcomes) => {
                    for o in outcomes {
                        if !o.sent.is_empty() {
                            log(format!("watch {}: sent {:?}", o.id, o.sent));
                        }
                        if !o.errors.is_empty() {
                            log(format!("watch {}: {}", o.id, o.errors.join("; ")));
                        }
                    }
                }
                Err(e) => log(format!("watchtower round failed: {e}")),
            }
        }
    });

    let purge_store = store.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(PURGE_EVERY);
        loop {
            every.tick().await;
            match handlers::purge(&purge_store, now()) {
                Ok(p) => log(format!(
                    "purged {} offer(s), {} message(s), {} job(s)",
                    p.offers, p.messages, p.jobs
                )),
                Err(e) => log(format!("purge failed: {e}")),
            }
        }
    });

    let listen = cfg.listen;
    let app = Arc::new(App { store, cfg });
    let router = Router::new().fallback(handle).with_state(app);
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|e| format!("cannot listen on {listen}: {e}"))?;
    log(format!("listening on {listen}"));
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|e| format!("server: {e}"))?;
    log("stopped");
    Ok(())
}

async fn handle(State(app): State<Arc<App>>, req: Request) -> Response {
    let started = Instant::now();
    let method = req.method().as_str().to_owned();
    let path = req.uri().path().to_owned();
    let query = req.uri().query().unwrap_or("").to_owned();
    let declared = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    let too_large = || Reply {
        status: 413,
        body: serde_json::json!({
            "error": format!("the body is larger than {MAX_MESSAGE_BYTES} bytes")
        }),
    };
    let reply = if declared.is_some_and(|n| n > MAX_MESSAGE_BYTES) {
        too_large()
    } else {
        // A read error is a body over the limit or a client that went away;
        // the second never reads the answer.
        match axum::body::to_bytes(req.into_body(), MAX_MESSAGE_BYTES).await {
            Err(_) => too_large(),
            Ok(bytes) => match std::str::from_utf8(&bytes) {
                Err(_) => Reply {
                    status: 400,
                    body: serde_json::json!({ "error": "the body is not UTF-8" }),
                },
                Ok(body) => {
                    handlers::dispatch(&app.store, &app.cfg, &method, &path, &query, body, now())
                }
            },
        }
    };
    if reply.status >= 500 {
        log(format!(
            "{method} {path} {}: {}",
            reply.status, reply.body["error"]
        ));
    }
    log(format!(
        "{method} {path} {} {}ms",
        reply.status,
        started.elapsed().as_millis()
    ));
    // The handlers answer only valid statuses and JSON values; these
    // fallbacks cannot be reached and only keep the binding total.
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = serde_json::to_vec(&reply.body).unwrap_or_else(|_| b"{}".to_vec());
    let mut resp = Response::new(Body::from(body));
    *resp.status_mut() = status;
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    resp
}

/// SIGINT (Ctrl+C) or SIGTERM: stop accepting, finish the requests under way.
async fn shutdown() {
    let interrupt = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = interrupt => {}
        _ = terminate => {}
    }
    log("shutting down");
}
