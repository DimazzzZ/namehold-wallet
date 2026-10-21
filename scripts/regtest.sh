#!/usr/bin/env bash
# regtest.sh — spin up a disposable Handshake (hsd) regtest node, fund/mine it,
# and run Namehold live-node integration tests against it, then tear it down.
#
# Everything lives under a repo-local, git-ignored data dir (.regtest/) so runs
# are throwaway and NEVER touch the user real ~/.hsd chain.
#
# Usage:
#   scripts/regtest.sh start              # launch node (idempotent), wait for RPC
#   scripts/regtest.sh start --with-wallet  # same, but with the hsd wallet enabled
#   scripts/regtest.sh --with-wallet      # shorthand for the line above
#   scripts/regtest.sh stop               # graceful stop, fall back to pid kill
#   scripts/regtest.sh reset              # stop + wipe .regtest/ (fresh chain)
#   scripts/regtest.sh fund <addr> [n]    # mine n blocks (default 110) to <addr>
#   scripts/regtest.sh mine [n] [addr]    # mine n blocks (default 1)
#   scripts/regtest.sh rpc <method> [..]  # hsd-cli rpc passthrough
#   scripts/regtest.sh run-it             # start (idempotent) + run live suite
#   scripts/regtest.sh start-noaddrindex  # an index-less node (no
#                                         # --index-tx, no --index-address)
#   scripts/regtest.sh stop-noaddrindex   # stop that node
#   scripts/regtest.sh start-cpfp         # an indexed node whose blocks hold
#                                         # only a few transactions (Speed up)
#   scripts/regtest.sh stop-cpfp          # stop that node
#
# Requires: hsd + hsd-cli on PATH (brew install hsd, or npm i -g hsd).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REGTEST_DIR="$REPO_ROOT/.regtest"
PID_FILE="$REGTEST_DIR/hsd.pid"

RPC_HOST="127.0.0.1"
RPC_PORT="14037"
RPC_URL="http://$RPC_HOST:$RPC_PORT"
API_KEY="test"
NETWORK="regtest"
WALLET_PORT="14039"
WALLET_URL="http://$RPC_HOST:$WALLET_PORT"
# Set by --with-wallet: leave the hsd wallet plugin enabled (default: --no-wallet).
WITH_WALLET=0

# A node without --index-tx and --index-address, for the live_noaddrindex_*
# tests: its own ports and data dir, its own chain (it does not connect to the
# indexed node).
NOADDR_DIR="$REGTEST_DIR/noaddrindex"
NOADDR_PID_FILE="$NOADDR_DIR/hsd.pid"
NOADDR_PORT="34037"
NOADDR_URL="http://$RPC_HOST:$NOADDR_PORT"

# An indexed node whose block template holds only a few transactions, for
# the Speed up live test (live_cpfp_*): `--max-weight` caps the template
# (hsd lib/node/fullnode.js passes it to the miner, lib/mining/miner.js),
# whose weight starts at the 4000 reserved for the coinbase. 5700 leaves room
# for three 1-in-2-out sends (557 weight units each) and not a fourth. Its own
# ports and data dir, its own chain (it does not connect to the other nodes).
CPFP_DIR="$REGTEST_DIR/cpfp"
CPFP_PID_FILE="$CPFP_DIR/hsd.pid"
CPFP_PORT="44037"
CPFP_URL="http://$RPC_HOST:$CPFP_PORT"
CPFP_MAX_WEIGHT="5700"

log()  { printf '[regtest] %s\n' "$*" >&2; }
die()  { printf '[regtest] error: %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "'$1' not found on PATH (install hsd: brew install hsd, or npm i -g hsd)"; }

# hsd-cli rpc wrapper.
cli() {
  need hsd-cli
  hsd-cli --network="$NETWORK" --api-key="$API_KEY" rpc "$@"
}

# True if the node answers RPC.
rpc_alive() {
  cli getblockchaininfo >/dev/null 2>&1
}

# Guard: refuse to operate on anything but the explicit repo-local .regtest dir.
assert_safe_regtest_dir() {
  case "$REGTEST_DIR" in
    ""|"/"|"$HOME"|"$HOME/") die "refusing to operate on unsafe path: '$REGTEST_DIR'" ;;
  esac
  [ "$REGTEST_DIR" = "$REPO_ROOT/.regtest" ] || die "REGTEST_DIR drifted from repo-local path: '$REGTEST_DIR'"
}

# True if the hsd wallet HTTP API answers (only up when started --with-wallet).
wallet_alive() {
  curl -fsS --max-time 2 -u "x:$API_KEY" "$WALLET_URL/" >/dev/null 2>&1
}

print_wallet_info() {
  log "wallet API: $WALLET_URL (port $WALLET_PORT, api-key=$API_KEY, wallet id: primary)"
}

cmd_start() {
  need hsd
  assert_safe_regtest_dir
  if rpc_alive; then
    log "node already responding on $RPC_URL"
    if [ "$WITH_WALLET" = 1 ]; then
      need curl
      wallet_alive || die "node is running without the wallet; run: regtest.sh stop, then start --with-wallet"
      print_wallet_info
    fi
    return 0
  fi
  mkdir -p "$REGTEST_DIR"
  local wallet_flags=(--no-wallet)
  [ "$WITH_WALLET" = 1 ] && wallet_flags=(--wallet-http-host="$RPC_HOST")
  log "starting hsd regtest node (prefix=$REGTEST_DIR, wallet=$([ "$WITH_WALLET" = 1 ] && echo on || echo off))"
  # --index-address --index-tx are REQUIRED and must be set from a fresh chain;
  # hsd cannot add an index to an existing chain (run reset if the dir was ever
  # created without them).
  hsd --network="$NETWORK" \
      --index-address --index-tx \
      --http-host="$RPC_HOST" --api-key="$API_KEY" \
      --prefix="$REGTEST_DIR" \
      "${wallet_flags[@]}" \
      --daemon
  pgrep -f "hsd .*--prefix=$REGTEST_DIR" | head -1 > "$PID_FILE" 2>/dev/null || true

  log "waiting for RPC on $RPC_URL ..."
  local tries=0
  until rpc_alive; do
    tries=$((tries + 1))
    [ "$tries" -ge 30 ] && die "node did not answer RPC within ~15s"
    sleep 0.5
  done
  log "node up on $RPC_URL (api-key=$API_KEY)"
  if [ "$WITH_WALLET" = 1 ]; then
    print_wallet_info
  fi
}

cmd_stop() {
  if rpc_alive; then
    log "stopping node via RPC"
    cli stop >/dev/null 2>&1 || true
    local tries=0
    while rpc_alive && [ "$tries" -lt 20 ]; do sleep 0.5; tries=$((tries + 1)); done
  fi
  if [ -f "$PID_FILE" ]; then
    local pid; pid="$(cat "$PID_FILE" 2>/dev/null || true)"
    if [ -n "${pid:-}" ] && kill -0 "$pid" 2>/dev/null; then
      log "pid $pid still alive; sending TERM"
      kill "$pid" 2>/dev/null || true
    fi
    rm -f "$PID_FILE"
  fi
  log "stopped"
}

# hsd-cli rpc against the index-less node.
noaddr_cli() {
  need hsd-cli
  hsd-cli --network="$NETWORK" --api-key="$API_KEY" --http-port="$NOADDR_PORT" rpc "$@"
}

noaddr_alive() {
  noaddr_cli getblockchaininfo >/dev/null 2>&1
}

cmd_start_noaddrindex() {
  need hsd
  assert_safe_regtest_dir
  if noaddr_alive; then
    log "index-less node already responding on $NOADDR_URL"
    return 0
  fi
  mkdir -p "$NOADDR_DIR"
  log "starting index-less hsd regtest node (prefix=$NOADDR_DIR)"
  # No --index-tx, no --index-address: hsd then answers GET /coin/address and
  # GET /tx/address with an empty list, never an error.
  hsd --network="$NETWORK" --no-wallet --listen=false \
      --http-host="$RPC_HOST" --http-port="$NOADDR_PORT" --port=34038 \
      --ns-port=35449 --rs-port=35450 --api-key="$API_KEY" \
      --prefix="$NOADDR_DIR" \
      --daemon
  pgrep -f "hsd .*--prefix=$NOADDR_DIR" | head -1 > "$NOADDR_PID_FILE" 2>/dev/null || true

  local tries=0
  until noaddr_alive; do
    tries=$((tries + 1))
    [ "$tries" -ge 30 ] && die "index-less node did not answer RPC within ~15s"
    sleep 0.5
  done
  log "index-less node up on $NOADDR_URL (api-key=$API_KEY)"
}

cmd_stop_noaddrindex() {
  if noaddr_alive; then
    log "stopping index-less node via RPC"
    noaddr_cli stop >/dev/null 2>&1 || true
    local tries=0
    while noaddr_alive && [ "$tries" -lt 20 ]; do sleep 0.5; tries=$((tries + 1)); done
  fi
  if [ -f "$NOADDR_PID_FILE" ]; then
    local pid; pid="$(cat "$NOADDR_PID_FILE" 2>/dev/null || true)"
    if [ -n "${pid:-}" ] && kill -0 "$pid" 2>/dev/null; then
      log "pid $pid still alive; sending TERM"
      kill "$pid" 2>/dev/null || true
    fi
    rm -f "$NOADDR_PID_FILE"
  fi
  log "index-less node stopped"
}

# hsd-cli rpc against the Speed up node.
cpfp_cli() {
  need hsd-cli
  hsd-cli --network="$NETWORK" --api-key="$API_KEY" --http-port="$CPFP_PORT" rpc "$@"
}

cpfp_alive() {
  cpfp_cli getblockchaininfo >/dev/null 2>&1
}

cmd_start_cpfp() {
  need hsd
  assert_safe_regtest_dir
  if cpfp_alive; then
    log "Speed up node already responding on $CPFP_URL"
    return 0
  fi
  mkdir -p "$CPFP_DIR"
  log "starting Speed up hsd regtest node (prefix=$CPFP_DIR, max-weight=$CPFP_MAX_WEIGHT)"
  hsd --network="$NETWORK" --no-wallet --listen=false \
      --index-address --index-tx \
      --max-weight="$CPFP_MAX_WEIGHT" \
      --http-host="$RPC_HOST" --http-port="$CPFP_PORT" --port=44038 \
      --ns-port=45449 --rs-port=45450 --api-key="$API_KEY" \
      --prefix="$CPFP_DIR" \
      --daemon > "$CPFP_PID_FILE"
  # `--daemon` prints the node's pid; hsd renames its process to "hsd", so
  # `pgrep -f` on the prefix finds nothing.
  grep -qE '^[0-9]+$' "$CPFP_PID_FILE" || die "hsd --daemon printed no pid: $(cat "$CPFP_PID_FILE")"

  local tries=0
  until cpfp_alive; do
    tries=$((tries + 1))
    [ "$tries" -ge 30 ] && die "Speed up node did not answer RPC within ~15s"
    sleep 0.5
  done
  log "Speed up node up on $CPFP_URL (api-key=$API_KEY)"
}

cmd_stop_cpfp() {
  local pid=""
  [ -f "$CPFP_PID_FILE" ] && pid="$(cat "$CPFP_PID_FILE" 2>/dev/null || true)"
  if cpfp_alive; then
    log "stopping Speed up node via RPC"
    cpfp_cli stop >/dev/null 2>&1 || true
  fi
  # hsd answers RPC for a while after `stop`: wait for the process itself, so
  # a start right after finds the data dir free (the live test restarts it).
  local tries=0
  while [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null && [ "$tries" -lt 40 ]; do
    sleep 0.5; tries=$((tries + 1))
  done
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    log "pid $pid still alive; sending TERM"
    kill "$pid" 2>/dev/null || true
    sleep 1
  fi
  rm -f "$CPFP_PID_FILE"
  log "Speed up node stopped"
}

cmd_reset() {
  assert_safe_regtest_dir
  cmd_stop_cpfp
  cmd_stop_noaddrindex
  cmd_stop
  if [ -d "$REGTEST_DIR" ]; then
    log "removing data dir $REGTEST_DIR"
    rm -rf "$REGTEST_DIR"
  fi
  log "reset complete (fresh chain on next start)"
}

cmd_fund() {
  local addr="${1:-}"; local n="${2:-110}"
  [ -n "$addr" ] || die "usage: regtest.sh fund <address> [nblocks]"
  rpc_alive || die "node not running (run: regtest.sh start)"
  log "mining $n blocks to $addr (regtest coinbase matures after 2)"
  cli generatetoaddress "$n" "$addr" >/dev/null
  log "funded"
}

cmd_mine() {
  local n="${1:-1}"; local addr="${2:-}"
  rpc_alive || die "node not running (run: regtest.sh start)"
  if [ -z "$addr" ]; then
    addr="$(cli getnewaddress 2>/dev/null || true)"
    [ -n "$addr" ] || die "no address given and node has --no-wallet; pass an address: regtest.sh mine $n <addr>"
  fi
  log "mining $n blocks to $addr"
  cli generatetoaddress "$n" "$addr" >/dev/null
}

cmd_rpc() {
  [ "$#" -ge 1 ] || die "usage: regtest.sh rpc <method> [args...]"
  cli "$@"
}

cmd_run_it() {
  need cargo
  cmd_start
  log "running live-node integration suite against $RPC_URL"
  HNS_IT_NODE_URL="$RPC_URL" \
  HNS_IT_NODE_API_KEY="$API_KEY" \
    cargo test --manifest-path "$REPO_ROOT/src-tauri/Cargo.toml" live_ \
      -- --nocapture --test-threads=1
}

main() {
  # --with-wallet may appear anywhere; strip it and, if it is the only
  # argument, imply the start subcommand.
  local args=() a
  for a in "$@"; do
    if [ "$a" = "--with-wallet" ]; then WITH_WALLET=1; else args+=("$a"); fi
  done
  set -- ${args[@]+"${args[@]}"}
  if [ "$WITH_WALLET" = 1 ] && [ "$#" -eq 0 ]; then set -- start; fi
  local sub="${1:-}"; shift || true
  case "$sub" in
    start)   cmd_start "$@" ;;
    stop)    cmd_stop "$@" ;;
    reset)   cmd_reset "$@" ;;
    fund)    cmd_fund "$@" ;;
    mine)    cmd_mine "$@" ;;
    rpc)     cmd_rpc "$@" ;;
    run-it)  cmd_run_it "$@" ;;
    start-noaddrindex) cmd_start_noaddrindex "$@" ;;
    stop-noaddrindex)  cmd_stop_noaddrindex "$@" ;;
    start-cpfp)        cmd_start_cpfp "$@" ;;
    stop-cpfp)         cmd_stop_cpfp "$@" ;;
    ""|-h|--help|help)
      sed -n '2,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      ;;
    *) die "unknown subcommand: '$sub' (try: start stop reset fund mine rpc run-it start-noaddrindex stop-noaddrindex start-cpfp stop-cpfp)" ;;
  esac
}

main "$@"
