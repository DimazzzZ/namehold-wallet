#!/usr/bin/env bash
# btc-regtest.sh — a disposable Bitcoin Core regtest node for the HNS <-> BTC
# exchange (docs/specs/2026-11-07-hns-btc-exchange.md), next to the hsd one
# from scripts/regtest.sh. Repo-local, git-ignored data dir: .regtest/bitcoin.
#
# Usage:
#   scripts/btc-regtest.sh start             # launch bitcoind (idempotent), wait for RPC
#   scripts/btc-regtest.sh stop              # graceful stop
#   scripts/btc-regtest.sh reset             # stop + wipe .regtest/bitcoin
#   scripts/btc-regtest.sh mine [n]          # mine n blocks (default 1) to the node's wallet
#   scripts/btc-regtest.sh fund <addr> <btc> # send <btc> from the node's wallet to <addr>, mine 1
#   scripts/btc-regtest.sh mocktime <unix>   # set the node's clock (blocks mined after it carry it)
#   scripts/btc-regtest.sh cli <args…>       # bitcoin-cli passthrough
#   scripts/btc-regtest.sh source            # print the source line for Exchange settings
#
# The app's Exchange settings take the source `bitcoind+http://127.0.0.1:18443`
# and the bitcoind login `namehold:regtest`.
#
# Requires: bitcoind and bitcoin-cli (Bitcoin Core 25 or newer) on PATH.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DATA_DIR="$REPO_ROOT/.regtest/bitcoin"
RPC_PORT=18443
RPC_USER=namehold
RPC_PASS=regtest
WALLET=qa

log() { printf '[btc-regtest] %s\n' "$*" >&2; }
die() { printf '[btc-regtest] error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "'$1' not found on PATH (install Bitcoin Core)"; }

cli() {
  need bitcoin-cli
  bitcoin-cli -regtest -datadir="$DATA_DIR" -rpcport="$RPC_PORT" \
    -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASS" "$@"
}

wcli() { cli -rpcwallet="$WALLET" "$@"; }

alive() { cli getblockchaininfo >/dev/null 2>&1; }

assert_safe_dir() {
  [ "$DATA_DIR" = "$REPO_ROOT/.regtest/bitcoin" ] || die "data dir drifted: '$DATA_DIR'"
}

cmd_start() {
  need bitcoind
  assert_safe_dir
  if alive; then
    log "bitcoind already answering on 127.0.0.1:$RPC_PORT"
    return 0
  fi
  mkdir -p "$DATA_DIR"
  bitcoind -regtest -datadir="$DATA_DIR" -daemon -txindex=1 -server=1 \
    -rpcbind=127.0.0.1 -rpcallowip=127.0.0.1 -rpcport="$RPC_PORT" \
    -rpcuser="$RPC_USER" -rpcpassword="$RPC_PASS" -fallbackfee=0.0001 \
    -listen=0 -dnsseed=0
  for _ in $(seq 1 60); do
    alive && break
    sleep 0.5
  done
  alive || die "bitcoind did not answer"
  if ! cli listwallets | grep -q "\"$WALLET\""; then
    cli loadwallet "$WALLET" >/dev/null 2>&1 || cli createwallet "$WALLET" >/dev/null
  fi
  local height
  height=$(cli getblockcount)
  if [ "$height" -lt 101 ]; then
    log "mining $((101 - height)) blocks so the wallet has spendable coins"
    wcli generatetoaddress "$((101 - height))" "$(wcli getnewaddress)" >/dev/null
  fi
  log "bitcoind up: source bitcoind+http://127.0.0.1:$RPC_PORT, login $RPC_USER:$RPC_PASS"
}

cmd_stop() {
  alive && cli stop >/dev/null || log "not running"
}

case "${1:-}" in
  start) cmd_start ;;
  stop) cmd_stop ;;
  reset) cmd_stop || true; sleep 1; assert_safe_dir; rm -rf "$DATA_DIR"; log "wiped $DATA_DIR" ;;
  mine) wcli generatetoaddress "${2:-1}" "$(wcli getnewaddress)" >/dev/null; log "height $(cli getblockcount)" ;;
  fund)
    [ $# -ge 3 ] || die "usage: fund <addr> <btc>"
    txid=$(wcli sendtoaddress "$2" "$3")
    wcli generatetoaddress 1 "$(wcli getnewaddress)" >/dev/null
    log "sent $3 BTC to $2 in $txid (mined)"
    ;;
  mocktime) [ $# -ge 2 ] || die "usage: mocktime <unix>"; cli setmocktime "$2"; log "mocktime $2" ;;
  cli) shift; cli "$@" ;;
  source) echo "bitcoind+http://127.0.0.1:$RPC_PORT" ;;
  *) sed -n '2,20p' "$0"; exit 2 ;;
esac
