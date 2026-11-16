#!/usr/bin/env bash
# nostr-relay.sh — disposable local Nostr relays (strfry in Docker) for the
# exchange's QA stand (docs/specs/2026-11-12-exchange-nostr-transport.md,
# docs/qa/procedures/nostr-relay.md). Repo-local, git-ignored data:
# .regtest/nostr/<port>.
#
# Usage:
#   scripts/nostr-relay.sh start [n]     # start n relays on 7777, 7778, … (default 1; idempotent)
#   scripts/nostr-relay.sh stop [port]   # stop one relay, or all of them
#   scripts/nostr-relay.sh status        # which relays run, and their URLs
#   scripts/nostr-relay.sh urls          # the running relays' URLs, one per line (for Exchange settings)
#   scripts/nostr-relay.sh reset         # stop all and wipe .regtest/nostr
#
# A relay listens on 127.0.0.1 only. Its events survive `stop` and `start`
# (the data dir stays); `reset` removes them.
#
# Requires: Docker running. The image is pinned by digest.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DATA_ROOT="$REPO_ROOT/.regtest/nostr"
IMAGE="ghcr.io/hoytech/strfry@sha256:36f1886d185a88ca57c66ebe52e6e9e8428dac2486eea0a5d50ff934f18b60c3"
FIRST_PORT=7777
PREFIX="namehold-qa-nostr"

log() { printf '[nostr-relay] %s\n' "$*" >&2; }
die() { printf '[nostr-relay] error: %s\n' "$*" >&2; exit 1; }

need_docker() {
  command -v docker >/dev/null 2>&1 || die "'docker' not found on PATH"
  docker info >/dev/null 2>&1 || die "Docker is not running (start Docker Desktop first)"
}

assert_safe_dir() {
  [ "$DATA_ROOT" = "$REPO_ROOT/.regtest/nostr" ] || die "data dir drifted: '$DATA_ROOT'"
}

running_ports() {
  docker ps --filter "name=^${PREFIX}-" --format '{{.Names}}' | sed "s/^${PREFIX}-//" | sort -n
}

# The image's own config, but listening on every interface inside the
# container (its default, 127.0.0.1, is unreachable through a published
# port) and without the file-descriptor limit a container may refuse.
write_config() {
  local conf="$DATA_ROOT/strfry.conf"
  [ -f "$conf" ] && return 0
  mkdir -p "$DATA_ROOT"
  docker run --rm --entrypoint cat "$IMAGE" /app/strfry.conf \
    | sed -e 's/^\( *bind = \)"127.0.0.1"/\1"0.0.0.0"/' -e 's/^\( *nofiles = \)[0-9]*/\10/' \
    > "$conf"
  grep -q 'bind = "0.0.0.0"' "$conf" || die "could not set bind in $conf"
}

cmd_start() {
  need_docker
  assert_safe_dir
  local n="${1:-1}"
  write_config
  for i in $(seq 0 $((n - 1))); do
    local port=$((FIRST_PORT + i))
    local name="${PREFIX}-${port}"
    if docker ps --format '{{.Names}}' | grep -qx "$name"; then
      log "relay already running: ws://127.0.0.1:$port"
      continue
    fi
    docker rm -f "$name" >/dev/null 2>&1 || true
    mkdir -p "$DATA_ROOT/$port"
    chmod 777 "$DATA_ROOT/$port"
    docker run -d --name "$name" \
      -p "127.0.0.1:${port}:7777" \
      -v "$DATA_ROOT/strfry.conf:/app/strfry.conf:ro" \
      -v "$DATA_ROOT/$port:/app/strfry-db" \
      "$IMAGE" >/dev/null
    for _ in $(seq 1 30); do
      if curl -s -m 2 -H 'Accept: application/nostr+json' "http://127.0.0.1:$port/" | grep -q supported_nips; then
        log "relay up: ws://127.0.0.1:$port"
        break
      fi
      sleep 1
    done
    curl -s -m 2 -H 'Accept: application/nostr+json' "http://127.0.0.1:$port/" | grep -q supported_nips \
      || die "relay on $port did not answer; see: docker logs $name"
  done
}

cmd_stop() {
  need_docker
  local ports
  if [ -n "${1:-}" ]; then ports="$1"; else ports="$(running_ports)"; fi
  for port in $ports; do
    docker rm -f "${PREFIX}-${port}" >/dev/null 2>&1 && log "stopped ws://127.0.0.1:$port"
  done
}

cmd_status() {
  need_docker
  local any=0
  for port in $(running_ports); do
    any=1
    printf 'ws://127.0.0.1:%s running\n' "$port"
  done
  [ "$any" = 1 ] || log "no relay running"
}

cmd_urls() {
  need_docker
  for port in $(running_ports); do printf 'ws://127.0.0.1:%s\n' "$port"; done
}

cmd_reset() {
  assert_safe_dir
  cmd_stop
  rm -rf "$DATA_ROOT"
  log "wiped $DATA_ROOT"
}

case "${1:-}" in
  start) shift; cmd_start "$@" ;;
  stop) shift; cmd_stop "$@" ;;
  status) cmd_status ;;
  urls) cmd_urls ;;
  reset) cmd_reset ;;
  *) sed -n '2,16p' "$0"; exit 2 ;;
esac
