# exchange-setup

Stand up everything the HNS ⇄ BTC exchange cases ([scenarios/exchange/](../scenarios/exchange/)) assume: the hsd regtest node, a Bitcoin Core regtest node, a local `namehold-relay` with its watchtower on, two regtest profiles (Buyer and Seller) with the exchange enabled and set for regtest, the Seller's HNS and the Buyer's BTC. The rules of [rules.md](../rules.md) apply throughout; the exchange's own requirements are in [environment.md](../environment.md#the-exchange-bitcoin-core-and-the-relay).

- **Parameters:** none.
- **Blocks mined:** HNS about 12 (the Seller's funding; more past height 20 000) plus whatever the hsd wallet needs; BTC 101 on a fresh bitcoind (the script's own), +1 for the Buyer's funding.
- **Mode:** B (isolated dev build) is the one this library assumes for the exchange: several cases quit and kill the app, stop its daemon and use the webview inspector. Mode A works for the rest; a step that needs the inspector is then ◐ or ✗ as the case says.

## Variables and helpers

Add these to the shell of [environment.md](../environment.md#variables) (which defines `REPO`, `QA_DIR`, `W`, `WADDR`, `mine`, `info`):

```bash
export REPO QA_DIR
RELAY=http://127.0.0.1:8787
B()     { bash "$REPO/scripts/btc-regtest.sh" cli "$@"; }         # bitcoin-cli on the exchange's bitcoind
BW()    { B -rpcwallet=qa "$@"; }                                  # its wallet "qa": pays the app, receives withdrawals
bmine() { bash "$REPO/scripts/btc-regtest.sh" mine "${1:-1}"; }   # mine n Bitcoin blocks to the "qa" wallet
H()     { hsd-cli --network=regtest --api-key=test rpc "$@"; }     # the same as scripts/regtest.sh rpc
sha()   { python3 -c 'import hashlib,sys; print(hashlib.sha256(bytes.fromhex(sys.argv[1])).hexdigest())' "$1"; }
watch_job() { curl -s "$RELAY/v1/watch/$1"; echo; }               # $1 = <terms id>-buyer or <terms id>-seller

# Both chains' median time past against the real clock.
mtp() { python3 - <<'PY'
import json, os, subprocess, time
h = json.loads(subprocess.check_output(["hsd-cli", "--network=regtest", "--api-key=test", "rpc", "getblockchaininfo"]))
b = json.loads(subprocess.check_output(["bash", os.environ["REPO"] + "/scripts/btc-regtest.sh", "cli", "getblockchaininfo"]))
now = int(time.time())
for name, i in (("hns", h), ("btc", b)):
    print(f"{name}: height {i['blocks']} mediantime {i['mediantime']} ({i['mediantime'] - now:+d} s from now)")
PY
}

# Every transaction at an HNS address (the node's address index, mempool included): what it spends, with witness, and what it pays (values in doos).
hns_at() { curl -s -u x:test "http://127.0.0.1:14037/tx/address/$1" | python3 -c '
import json, sys
for t in json.load(sys.stdin):
    print(t["hash"], "height", t["height"], "locktime", t["locktime"])
    for i in t["inputs"]:
        print("   in ", i["prevout"]["hash"] + ":" + str(i["prevout"]["index"]), "witness", i["witness"])
    for n, o in enumerate(t["outputs"]):
        print("   out", n, o["value"], o["address"])
'; }

# Every transaction in bitcoind's mempool or its last 30 blocks that spends <txid>:<vout>.
btc_spender() { python3 - "$1" "${2:-0}" <<'PY'
import json, os, subprocess, sys
txid, vout = sys.argv[1], int(sys.argv[2])
cli = ["bash", os.environ["REPO"] + "/scripts/btc-regtest.sh", "cli"]
def j(*a): return json.loads(subprocess.check_output(cli + list(a)))
cands = [(t, "mempool") for t in j("getrawmempool")]
tip = j("getblockcount")
for h in range(tip, max(tip - 30, 0), -1):
    bh = subprocess.check_output(cli + ["getblockhash", str(h)]).decode().strip()
    cands += [(t, f"height {h}") for t in j("getblock", bh, "1")["tx"]]
for t, where in cands:
    tx = j("getrawtransaction", t, "true")
    for i in tx["vin"]:
        if i.get("txid") == txid and i.get("vout") == vout:
            print(t, where, "locktime", tx["locktime"])
            print("   witness", i.get("txinwitness"))
            for o in tx["vout"]:
                print("   out", o["n"], o["value"], o["scriptPubKey"].get("address"))
PY
}

# A trade's exact times (Unix seconds), from the watchtower's copy of its terms; $1 = the terms id (the Commit's "Trade" row).
terms_times() { sqlite3 "$QA_DIR/relay/regtest.db" "select json from watch_jobs where id like '$1-%' limit 1" | python3 -c '
import json, sys
t = json.load(sys.stdin)["terms"]
for k in ("start", "buyerFundDeadline", "sellerFundDeadline", "revealDeadline", "hnsRefundTime", "btcRefundTime", "btcAmount", "hnsAmount"):
    print(k, t[k])
'; }
```

Where the secret sits: both lock scripts start `63 82 01 20 88 a8 20 <H, 32 bytes> 88 21 <claim key> 67 …` (spec §3.1), so in hex `H` is characters 15–78 of the script (`${SCRIPT:14:64}`). A claim's witness is `[signature, x, 01, script]`: item 1 is the 32-byte secret `x` (64 hex), item 2 is `01`, item 3 the script; `sha "$X"` must equal `H`. A refund's witness is `[signature, "", script]`. The Buyer's HNS claim signature ends in `c1` (`SIGHASH_ALL | NOINPUT | ANYONECANPAY`); every other signature ends in `01`.

## Steps

1. **hsd.** `scripts/regtest.sh --with-wallet` ([environment](../environment.md#regtest-node-and-hsd-wallet)); record `scripts/regtest.sh rpc getblockcount` (past 24 000: stop and ask). Check `W getbalance` has at least 10 HNS spendable (E24 pays 1 HNS from it); else `mine 10`, then `mine 2` more.
2. **bitcoind.** `bash scripts/btc-regtest.sh start`. Its last line reads `[btc-regtest] bitcoind up: source bitcoind+http://127.0.0.1:18443, login namehold:regtest`. `B getblockchaininfo` shows `"chain": "regtest"`. **Fee estimates.** A fresh chain has none: `B estimatesmartfee 2` answers `"errors": ["Insufficient data or no fee rate found"]`, and every BTC send of the exchange is then refused ([E36](../scenarios/exchange/btc-account.md#e36--no-fee-estimate-a-btc-send-is-refused-before-signing) checks that refusal; run it first when the chain is fresh). Seed the estimator: `A=$(BW getnewaddress); for b in $(seq 1 30); do for r in 2 5 10 20 40 80; do BW -named sendtoaddress address=$A amount=0.001 fee_rate=$r >/dev/null; done; bmine 1 >/dev/null; done`, until `B estimatesmartfee 2` answers a `feerate`. Check it again after every bitcoind restart and before each trade case: a Core without estimates blocks every Commit.
3. **Clocks.** `mtp`: both `mediantime`s must be at or before now (a negative or zero offset). A positive offset is left over from a mocktime of an earlier run ([exchange-time-travel](exchange-time-travel.md#after-a-jump)): no trade may start until both are back at or before now; wait, or ask the maintainer.
4. **The relay.** Build it from the branch under test, write its regtest configuration and start it:

   ```bash
   (cd "$REPO/src-tauri" && cargo build --release --features relay-server --bin namehold-relay)
   mkdir -p "$QA_DIR/relay" && chmod 700 "$QA_DIR/relay"
   cat > "$QA_DIR/relay/config.json" <<EOF
   {
     "listen": "127.0.0.1:8787",
     "dbPath": "$QA_DIR/relay/regtest.db",
     "networks": {
       "regtest": {
         "node": { "url": "http://127.0.0.1:14037", "apiKey": "test" },
         "btcSources": ["bitcoind+http://namehold:regtest@127.0.0.1:18443"]
       }
     }
   }
   EOF
   chmod 600 "$QA_DIR/relay/config.json"
   relay_start() { nohup "${CARGO_TARGET_DIR:-$REPO/src-tauri/target}/release/namehold-relay" "$QA_DIR/relay/config.json" >> "$QA_DIR/relay/relay.log" 2>&1 & echo $! > "$QA_DIR/relay/relay.pid"; sleep 1; }
   relay_stop()  { kill "$(cat "$QA_DIR/relay/relay.pid")"; }
   relay_start
   curl -s "$RELAY/v1/health"; echo
   ```

   `curl` answers `{"ok":true,"version":"<v>","networks":["regtest"]}`; `relay.log` holds a line ending `regtest: relay and watchtower (node http://127.0.0.1:14037, 1 Bitcoin source(s))` and one ending `listening on 127.0.0.1:8787`. The relay's database and log are the run's own files: reading them (`sqlite3`, `grep`) is allowed. Record the version.
5. **The app.** Mode B: the app starts the daemon that sits next to its own binary, and `tauri dev` copies the sidecar `src-tauri/binaries/namehold-syncd-<host triple>` over `debug/namehold-syncd` at every start. Build the daemon with the dev build's features and stage it there, or the app runs an old daemon that syncs profiles and silently lacks the exchange step: `(cd "$REPO/src-tauri" && cargo build --no-default-features --bin namehold-syncd && cp "$(cargo metadata --format-version 1 --no-deps | python3 -c 'import sys,json; print(json.load(sys.stdin)["target_directory"])')/debug/namehold-syncd" "binaries/namehold-syncd-$(rustc -vV | sed -n 's/^host: //p')")`. Then start the dev build ([environment](../environment.md#modes)). Check after step 7: `strings <the running daemon's path> | grep -c v1/offers` is at least 1, and its lines in `~/.namehold/syncd.log` start with a UTC timestamp. Keep the binaries the run uses on a disk that stays mounted: when a disk holding a running binary is re-enumerated, the app, the daemon and the relay spin at about 100 % CPU and answer nothing (`ps -o %cpu`; the relay's `curl` times out); kill and start them again. A long run is safer from copies on the internal disk: the app binary started directly with the run's `HOME` while `pnpm dev` serves the frontend, the daemon copied next to it, the relay from its own copy. Nothing may rebuild or replace a binary while its process runs.
6. **Profiles.** Create two regtest profiles with **different** recovery phrases ([environment › Profiles](../environment.md#profiles), steps 1–2): **Seller** (example name `QA Exchange Seller`) and **Buyer** (`QA Exchange Buyer`). An existing seller profile of the Shakedex roles may serve as the Seller; the Buyer must not share its phrase. The watch-only profile E1 needs is made in E1. The exchange refuses a remote node (R17, [gates](../scenarios/exchange/gates-and-settings.md)), so connect both to the regtest node as a node on this computer, as [REGTEST_TESTING.md](../../REGTEST_TESTING.md) §3 does: Settings › Connections → `Chain source` `Local full node (this device runs hsd)`, `Node RPC URL (sending)` `http://127.0.0.1:14037`, `Node RPC API key` `test` → `Save settings` → confirm in the secure window. Start the node (step 1) before the app, so the app adopts it rather than starting its own. Note the Connections settings each profile had before; if they change for other regtest profiles too, restore them at the end of the run.
7. **Background sync.** Settings › Node & tray: `Sync in background (keep wallet up to date when app is closed)` ticked (toast "Background sync enabled — daemon started" if you tick it). `pgrep -fl namehold-syncd` shows the daemon from this build (mode B: a path ending `debug/namehold-syncd`; `ps -o lstart= -p <pid>` later than step 5). The daemon steps **every** profile's trades every 60 s; the app's own sync (every 60 s, and `Refresh` on the Exchange page) steps only the **active** profile. Several cases rely on that difference.
8. **Enable the exchange on the Seller.** Sidebar `Exchange` (hotkey `4`). On a fresh setup run [E4](../scenarios/exchange/gates-and-settings.md#e4--not-enabled-yet-and-enabling) here; otherwise: unlock the signer (readiness pill › Unlock, passphrase in the secure window), then `Enable exchange` in the blue banner. Toast "Exchange enabled"; the banner is gone.
9. **Exchange settings.** On a fresh setup run [E6](../scenarios/exchange/gates-and-settings.md#e6--the-exchange-settings-dialog-and-its-secure-window) here; otherwise: `Settings` (top right of the Exchange page) → `Relay URL` `http://127.0.0.1:8787`, `Watchtower URL (optional)` `http://127.0.0.1:8787`, `Bitcoin sources, one per line` `bitcoind+http://127.0.0.1:18443`, `bitcoind RPC login (user:password)` `namehold:regtest` → `Save` → secure window `Change exchange settings` → `Confirm & Sign`. Toast "Exchange settings saved".
10. **Enable on the Buyer.** [switch-profile](switch-profile.md) to the Buyer, unlock, Exchange → `Enable exchange` → "Exchange enabled". Open `Settings`: the dialog shows the same relay, watchtower and source, the login field's placeholder `set` and "A user:password is set. Leave the field empty to keep it." (the exchange settings are stored per network for the whole app, not per profile: `exchange_relay_url.regtest` and its siblings). `Cancel`.
11. **The Buyer's BTC.** The `BTC account` card (right column) shows `Receive address` `bcrt1q…`. `bash scripts/btc-regtest.sh fund <that address> 0.01` (it mines 1 block; its log names the txid). Press `Refresh` (top right): `Confirmed` `0.01000000 BTC` / `1,000,000 sats`. The card has no timer of its own: it rereads on `Refresh`, on a Commit or a withdrawal, and when the page is opened again.
12. **The Seller's HNS.** [switch-profile](switch-profile.md) to the Seller; Wallet › `Receive` → `Your address` (`rs1…`); `mine 12 <rs1…>`, then **Sync**. Spendable must be at least 60 HNS (a happy path locks 5 HNS, a full run about 40 HNS plus fees); past height 20 000 a block pays under 8 HNS, so mine more if needed (`mine 2` more after the last funding block for maturity).
13. **Record**: both chains' heights and `mtp`, the relay version, the two profile names, the Buyer's BTC balance and the Seller's spendable HNS.

## Roles used by the cases

| Role | What | Made in |
|---|---|---|
| OS | the Seller's sell offer: price `10000` sats per HNS, smallest `1`, largest `20` HNS (each trade 10 000 to 200 000 sats) | [E10](../scenarios/exchange/offers-and-taking.md#e10--publish-a-sell-offer) |
| OX | the Seller's second sell offer at an odd price: `333.333`, smallest `5`, largest `20` HNS | E10 |
| OB | the Buyer's buy offer: price `10000`, smallest `1`, largest `20` HNS | [E11](../scenarios/exchange/offers-and-taking.md#e11--publish-a-buy-offer) |
| `HNS_LOCK`, `BTC_LOCK` | a trade's lock addresses, read from the Commit rows `HNS lock` and `BTC lock` | each trade |
| `TERMS` | a trade's terms id, the Commit row `Trade` (64 hex); the watchtower's jobs are `$TERMS-buyer` and `$TERMS-seller` | each trade |

A trade of 5 HNS on OS moves exactly 50 000 sats (`floor(doos × msat per HNS / 10^9)`, spec §9); 2.5 HNS moves 25 000 sats.

## End state

hsd and bitcoind running with both clocks at or before now; `namehold-relay` running (pid in `$QA_DIR/relay/relay.pid`); the app running with two enabled profiles and the regtest exchange settings; the daemon running; the Buyer holding 0.01 BTC, the Seller at least 60 HNS. Leave hsd, bitcoind and the relay running between cases; stop the relay (`relay_stop`) and bitcoind (`bash scripts/btc-regtest.sh stop`) at the end of the run. Never `reset` hsd ([rules](../rules.md#the-regtest-chain)); `bash scripts/btc-regtest.sh reset` wipes the exchange's own Bitcoin chain and is allowed only between runs, with every trade of the run finished.
