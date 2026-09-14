#!/usr/bin/env bash
# regtest.sh — spin up a disposable Handshake (hsd) regtest node, fund/mine it,
# and run Namehold live-node integration tests against it, then tear it down.
#
# Everything lives under a repo-local, git-ignored data dir (.regtest/) so runs
# are throwaway and NEVER touch the user real ~/.hsd chain.
#
# Usage:
#   scripts/regtest.sh start              # launch node (idempotent), wait for RPC
#   scripts/regtest.sh stop               # graceful stop, fall back to pid kill
#   scripts/regtest.sh reset              # stop + wipe .regtest/ (fresh chain)
#   scripts/regtest.sh fund <addr> [n]    # mine n blocks (default 110) to <addr>
#   scripts/regtest.sh mine [n] [addr]    # mine n blocks (default 1)
#   scripts/regtest.sh rpc <method> [..]  # hsd-cli rpc passthrough
#   scripts/regtest.sh run-it             # start (idempotent) + run live suite
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

cmd_start() {
  need hsd
  assert_safe_regtest_dir
  if rpc_alive; then
    log "node already responding on $RPC_URL"
    return 0
  fi
  mkdir -p "$REGTEST_DIR"
  log "starting hsd regtest node (prefix=$REGTEST_DIR)"
  # --index-address --index-tx are REQUIRED and must be set from a fresh chain;
  # hsd cannot add an index to an existing chain (run reset if the dir was ever
  # created without them).
  hsd --network="$NETWORK" \
      --index-address --index-tx \
      --http-host="$RPC_HOST" --api-key="$API_KEY" \
      --prefix="$REGTEST_DIR" \
      --no-wallet \
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

cmd_reset() {
  assert_safe_regtest_dir
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
  local sub="${1:-}"; shift || true
  case "$sub" in
    start)   cmd_start "$@" ;;
    stop)    cmd_stop "$@" ;;
    reset)   cmd_reset "$@" ;;
    fund)    cmd_fund "$@" ;;
    mine)    cmd_mine "$@" ;;
    rpc)     cmd_rpc "$@" ;;
    run-it)  cmd_run_it "$@" ;;
    ""|-h|--help|help)
      sed -n '2,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      ;;
    *) die "unknown subcommand: '$sub' (try: start stop reset fund mine rpc run-it)" ;;
  esac
}

main "$@"
