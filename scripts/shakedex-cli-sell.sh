#!/usr/bin/env bash
# shakedex-cli-sell.sh — drive the real shakedex CLI on regtest, so Namehold
# can buy what it lists (spec R30, "CLI sells, Namehold buys") and meet it as
# the other party: a buyer who comes first, a seller who cancels.
#
# Needs a wallet-enabled regtest node:  scripts/regtest.sh --with-wallet
#
# Usage:
#   shakedex-cli-sell.sh [fixed]        lock NAME and list it at PRICE (default)
#   shakedex-cli-sell.sh auction        lock NAME and list a reverse auction
#                                       from START_PRICE down to END_PRICE over
#                                       one day, one step every 15 minutes
#   shakedex-cli-sell.sh cancel NAME    take NAME back out of its lock
#                                       (transfer-lock-cancel, then
#                                       finalize-lock-cancel); needs the
#                                       SHAKEDEX_WORK the listing was made in
#   shakedex-cli-sell.sh fill LISTING   buy LISTING with the hsd wallet
#   shakedex-cli-sell.sh register       register NAME (default: a fresh name)
#                                       with the hsd wallet; prints the name
#   shakedex-cli-sell.sh lock-to NAME ADDRESS
#                                       transfer NAME to ADDRESS and finalize it
#                                       there, as a lock the caller holds the key
#                                       of; prints the name's owner outpoint
#                                       ("txid index"). Neither needs the CLI.
#
# The seller and the CLI buyer are the hsd wallet of that node. The shakedex
# CLI is cloned at a pinned SHA into a work dir (nothing is installed
# globally) and keeps its own database in a prefix there.
#
# Environment:
#   NAME       name to sell (must be registered to the hsd wallet); with
#              REGISTER=1 it defaults to a fresh unique name
#   REGISTER   1 = first register NAME with the hsd wallet (open, bid, reveal,
#              register, mining between the phases); default 0
#   PRICE      fixed price in HNS (default 5)
#   START_PRICE, END_PRICE   reverse auction prices in whole HNS (default 10, 5)
#   SHAKEDEX_WORK            work dir to keep the checkout and the CLI's
#              database in, reused across runs (default: a fresh temp dir,
#              kept after `fixed`/`auction`, removed after `fill`). A lock can
#              only be cancelled from the work dir it was made in, so `cancel`
#              needs it set.
#   OUT        where to write the listing file (default: ./shakedex-listing-$NAME.json)
#   HSD_API_KEY, WALLET_ID   node/wallet API key (default test) and wallet id (default primary)
#   TRANSFER_LOCKUP          regtest transfer lockup in blocks (default 10)
#   HSD_NPM_VERSION          hsd version installed next to the CLI (default: `hsd --version`)
#
# `fixed` and `auction` print the listing path on the last line of stdout.
set -euo pipefail

SHAKEDEX_REPO="https://github.com/shadstoneofficial/shakedex"
# The full commit hash: an abbreviation is not a pin, since a later commit can
# share its prefix and make the checkout ambiguous or different.
SHAKEDEX_SHA="2c4fa04eab68a528e758598d11b5da5666113b11"
NETWORK="regtest"

API_KEY="${HSD_API_KEY:-test}"
WALLET_ID="${WALLET_ID:-primary}"
PRICE="${PRICE:-5}"
START_PRICE="${START_PRICE:-10}"
END_PRICE="${END_PRICE:-5}"
MODE="${1:-fixed}"
REGISTER="${REGISTER:-0}"
TRANSFER_LOCKUP="${TRANSFER_LOCKUP:-10}"

log() { printf '[shakedex-sell] %s\n' "$*" >&2; }
die() { printf '[shakedex-sell] error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "'$1' not found on PATH"; }

need git
need node
need npm
need hsd-rpc
need hsw-rpc

case "$MODE" in
  fixed | auction) ;;
  cancel)
    NAME="${2:?usage: shakedex-cli-sell.sh cancel NAME}"
    [ -n "${SHAKEDEX_WORK:-}" ] || die "cancel needs SHAKEDEX_WORK, the work dir the lock was made in"
    ;;
  fill) LISTING="${2:?usage: shakedex-cli-sell.sh fill LISTING}" ;;
  register) NAME="${NAME:-shkreg$RANDOM$RANDOM}" ;;
  lock-to)
    NAME="${2:?usage: shakedex-cli-sell.sh lock-to NAME ADDRESS}"
    LOCK_ADDR="${3:?usage: shakedex-cli-sell.sh lock-to NAME ADDRESS}"
    ;;
  *) die "unknown command '$MODE' (fixed, auction, cancel NAME, fill LISTING, register, lock-to NAME ADDRESS)" ;;
esac

if [ "$MODE" = fixed ] || [ "$MODE" = auction ]; then
  if [ "$REGISTER" = 1 ] && [ -z "${NAME:-}" ]; then
    NAME="shkcli$RANDOM$RANDOM"
  fi
  [ -n "${NAME:-}" ] || die "set NAME (a name owned by the hsd wallet) or REGISTER=1"
  OUT="${OUT:-$PWD/shakedex-listing-$NAME.json}"
fi

rpc() { hsd-rpc --network="$NETWORK" --api-key="$API_KEY" "$@"; }
wrpc() { hsw-rpc --network="$NETWORK" --api-key="$API_KEY" "$@"; }

# One field of the JSON document on stdin, by dotted path ("info.owner.hash");
# empty when it is missing or null. node is already required by the CLI.
json() {
  node -e '
    let s = "";
    process.stdin.on("data", (d) => (s += d)).on("end", () => {
      let v = JSON.parse(s);
      for (const k of process.argv[1].split(".")) v = v == null ? v : v[k];
      process.stdout.write(v == null ? "" : String(v));
    });' "$1"
}

# Refuse anything that does not look like the throwaway regtest node. The
# reply is read whole: a grep that stops early would break the pipe.
CHAIN="$(rpc getblockchaininfo | json chain)" || die "no node answering (run: scripts/regtest.sh --with-wallet)"
[ "$CHAIN" = regtest ] || die "the node answering is on '$CHAIN', not regtest"

MINE_ADDR="$(wrpc getnewaddress | tr -d '"[:space:]')" ||
  die "hsd wallet gave no address (start the node with: scripts/regtest.sh --with-wallet)"
[ -n "$MINE_ADDR" ] || die "hsd wallet gave no address (start the node with: scripts/regtest.sh --with-wallet)"

mine() { rpc generatetoaddress "$1" "$MINE_ADDR" >/dev/null; }

name_field() { rpc getnameinfo "$NAME" | json "info.$1"; }
name_state() { name_field state; }

# The name's owner coin pays an address of the hsd wallet.
owner_is_ours() {
  local hash index addr
  hash="$(name_field owner.hash)"
  index="$(name_field owner.index)"
  addr="$(rpc gettxout "$hash" "$index" | json address.string)"
  [ -n "$addr" ] && [ "$(wrpc getaddressinfo "$addr" | json ismine)" = true ]
}

mine_until_state() {
  local want="$1" tries=0
  until [ "$(name_state)" = "$want" ]; do
    tries=$((tries + 1))
    [ "$tries" -gt 60 ] && die "$NAME did not reach $want"
    mine 1
  done
}

# Regtest auction params are tiny: a handful of blocks per phase.
register_name() {
  log "registering $NAME with the hsd wallet"
  # Fund the wallet (coinbases mature after 2 blocks on regtest), in steps:
  # one call mining 110 blocks can outlast hsd-rpc's request timeout.
  for _ in 1 2 3 4 5 6 7 8 9 10 11; do mine 10; done
  wrpc sendopen "$NAME" >/dev/null
  mine_until_state BIDDING
  # Two bids: the winner pays the SECOND price, so the lock coin is worth
  # 0.5 HNS. A lone bid (a 0-doo lock coin) is accepted too; two keep the
  # scenario's amounts non-trivial.
  wrpc sendbid "$NAME" 1 2 >/dev/null
  wrpc sendbid "$NAME" 0.5 2 >/dev/null
  mine_until_state REVEAL
  wrpc sendreveal "$NAME" >/dev/null
  mine_until_state CLOSED
  wrpc sendupdate "$NAME" '{"records":[{"type":"TXT","txt":["shakedex-cli-sell"]}]}' >/dev/null
  mine 1
  log "$NAME registered"
}

if [ "$MODE" = fixed ] || [ "$MODE" = auction ]; then
  [ "$REGISTER" = 1 ] && register_name
  [ "$(name_state)" = CLOSED ] || die "$NAME is not registered (state: $(name_state)); use REGISTER=1"
fi

case "$MODE" in
  register)
    register_name
    [ "$(name_state)" = CLOSED ] || die "$NAME did not register (state: $(name_state))"
    printf '%s\n' "$NAME"
    exit 0
    ;;
  lock-to)
    [ "$(name_state)" = CLOSED ] || die "$NAME is not registered (state: $(name_state))"
    log "transfer $NAME to $LOCK_ADDR"
    wrpc sendtransfer "$NAME" "$LOCK_ADDR" >/dev/null
    mine $((TRANSFER_LOCKUP + 1))
    log "finalize $NAME at $LOCK_ADDR"
    wrpc sendfinalize "$NAME" >/dev/null
    mine 1
    owner_hash="$(name_field owner.hash)"
    owner_index="$(name_field owner.index)"
    addr="$(rpc gettxout "$owner_hash" "$owner_index" | json address.string)"
    [ "$addr" = "$LOCK_ADDR" ] || die "$NAME's owner coin is at '$addr', not $LOCK_ADDR"
    printf '%s %s\n' "$owner_hash" "$owner_index"
    exit 0
    ;;
esac

WORK="${SHAKEDEX_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/shakedex-cli.XXXXXX")}"
mkdir -p "$WORK"
# A temp work dir is worth keeping only for the lock a listing was made from.
if [ -z "${SHAKEDEX_WORK:-}" ] && [ "$MODE" = fill ]; then
  trap 'rm -rf "$WORK"' EXIT
fi
# Reused only when a previous run finished the install: the checkout is at
# the pin and the marker written after `npm install` is there.
INSTALLED="$WORK/shakedex/.namehold-installed"
if [ "$(git -C "$WORK/shakedex" rev-parse HEAD 2>/dev/null)" != "$SHAKEDEX_SHA" ] ||
  [ ! -f "$INSTALLED" ]; then
  rm -rf "$WORK/shakedex"
  log "cloning $SHAKEDEX_REPO at $SHAKEDEX_SHA into $WORK"
  git clone --quiet "$SHAKEDEX_REPO" "$WORK/shakedex"
  git -C "$WORK/shakedex" checkout --quiet "$SHAKEDEX_SHA"
  [ "$(git -C "$WORK/shakedex" rev-parse HEAD)" = "$SHAKEDEX_SHA" ] ||
    die "shakedex checkout is not the pinned $SHAKEDEX_SHA"
  (
    cd "$WORK/shakedex"
    npm ci --no-audit --no-fund >&2
    # hsd is a peer requirement of the CLI (`npm i -g hsd` upstream); keep it local.
    npm install --no-save --no-audit --no-fund "hsd@${HSD_NPM_VERSION:-$(hsd --version)}" >&2
  )
  touch "$INSTALLED"
fi

# fill-auction draws progress with `process.stdout.clearLine`, which only a
# terminal has; our output goes to stderr and log files. A no-op stand-in,
# loaded before the CLI, leaves the CLI itself unchanged.
TTY_SHIM="$WORK/no-tty-shim.js"
cat >"$TTY_SHIM" <<'JS'
for (const s of [process.stdout, process.stderr]) {
  if (typeof s.clearLine !== 'function') s.clearLine = () => true;
  if (typeof s.cursorTo !== 'function') s.cursorTo = () => true;
}
JS

shakedex() {
  node --require "$TTY_SHIM" "$WORK/shakedex/bin/shakedex" \
    --prefix "$WORK/prefix" -n "$NETWORK" -w "$WALLET_ID" -a "$API_KEY" --no-passphrase "$@"
}
mkdir -p "$WORK/prefix"

# Write each argument as one line of input, a second apart, then keep the
# pipe open a moment so the last prompt can finish. A spare answer the CLI
# never asked for is dropped: once it has exited, the write fails quietly
# instead of failing the pipeline.
answers() (
  trap '' PIPE
  for a in "$@"; do
    sleep 1
    printf '%s\n' "$a" 2>/dev/null || exit 0
  done
  sleep 2
)

# The CLI asks for confirmation before each money-moving step.
lock_name() {
  log "transfer-lock $NAME"
  printf 'y\n' | shakedex transfer-lock "$NAME" >&2
  mine $((TRANSFER_LOCKUP + 2))
  log "finalize-lock $NAME"
  printf 'y\n' | shakedex finalize-lock "$NAME" >&2
  mine 1
}

case "$MODE" in
  fixed)
    lock_name
    log "create-fixed $NAME $PRICE HNS"
    shakedex create-fixed "$NAME" "$PRICE" -o "$OUT" >&2
    ;;
  auction)
    lock_name
    log "create-auction $NAME $START_PRICE -> $END_PRICE HNS"
    # Answers in prompt order: duration (first choice, 1 day), step interval
    # (first choice, every 15 minutes), start price, end price, output path,
    # "no" to publishing on LearnHNS, and "yes" to the price table it shows.
    # One at a time: inquirer reads what is buffered into the prompt it is
    # on, and closes when input ends.
    answers "" "" "$START_PRICE" "$END_PRICE" "$OUT" n y |
      shakedex create-auction "$NAME" >&2
    ;;
  cancel)
    log "transfer-lock-cancel $NAME"
    # A second confirmation follows when a listing of the lock exists.
    answers y y | shakedex transfer-lock-cancel "$NAME" >&2
    mine $((TRANSFER_LOCKUP + 2))
    log "finalize-lock-cancel $NAME"
    answers y y | shakedex finalize-lock-cancel "$NAME" >&2
    mine 1
    # The CLI exits 0 also when it did nothing: check the chain instead.
    [ "$(name_field transfer)" = 0 ] && owner_is_ours ||
      die "$NAME is not back in the hsd wallet after the cancel"
    log "$NAME is out of its lock"
    exit 0
    ;;
  fill)
    log "fill-auction $LISTING"
    NAME="$(json name <"$LISTING")"
    [ -n "$NAME" ] || die "no name in $LISTING"
    answers y y | shakedex fill-auction "$LISTING" >&2
    mine 1
    # A mined fill leaves the name in a TRANSFER out of the lock.
    [ "$(name_field transfer)" -gt 0 ] 2>/dev/null ||
      die "$NAME is not being transferred after the fill"
    log "$LISTING filled by the hsd wallet"
    exit 0
    ;;
esac

[ -s "$OUT" ] || die "listing file was not written: $OUT"
log "listing for $NAME written; shakedex work dir kept at $WORK"
printf '%s\n' "$OUT"
