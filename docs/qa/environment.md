# QA environment

How to stand up what the scenarios assume: a build of the branch under test, the regtest node with its hsd wallet, the Shakedex CLI seller, funded regtest profiles and a per-run set of test names. The rules in [rules.md](rules.md) apply throughout. The node script and its live tests are described in [REGTEST_TESTING.md](../REGTEST_TESTING.md); this file only adds what a GUI run needs.

## Variables

```bash
REPO=<the repo checkout>
QA_DIR=<a scratch dir outside the repo, or under the gitignored $REPO/.qa/>   # shots, notes, listing files
export SHAKEDEX_WORK=$QA_DIR/shakedex-work   # the CLI checkout and its database, reused by every CLI call
TAG=$(date +%m%d%H%M)                        # per-run suffix for test names
mkdir -p "$QA_DIR/shots" "$SHAKEDEX_WORK"
```

## Build and install from the branch head

A run tests one build, recorded in the report header ([report-template](report-template.md)). Build it first, from a clean tree:

```bash
cd $REPO
git rev-parse --short HEAD     # record it
git status --short             # only untracked files are allowed
osascript -e 'tell application "Namehold" to quit'
pnpm tauri build               # set CARGO_HOME / CARGO_TARGET_DIR the way the maintainer's machine does
```

- **The build takes the working tree, not the commit.** If `git status --short` shows modified tracked files, never stash, reset or commit them: build from a clean detached worktree instead (`git worktree add --detach $QA_DIR/build HEAD`, then `pnpm install --frozen-lockfile` and `pnpm tauri build` there, take the bundle, then `git worktree remove $QA_DIR/build`). Record which way you built.
- If the cargo target directory lives on an external volume and that volume is not mounted, stop and ask.
- The build may end with an error at the updater signing step (no private key in the environment). The `.app` is still produced: check that `$CARGO_TARGET_DIR/release/bundle/macos/Namehold.app` has a fresh mtime.
- **Install** (mode A): keep every earlier backup as it is; move the current app aside with `mv /Applications/Namehold.app /Applications/Namehold.app.qa-bak-<yyyymmdd>`, then `ditto $CARGO_TARGET_DIR/release/bundle/macos/Namehold.app /Applications/Namehold.app`.
- **Record**: the commit, `shasum -a 256` of `/Applications/Namehold.app/Contents/MacOS/namehold-wallet` and of `$CARGO_TARGET_DIR/release/namehold-wallet` (they must match), the binary's mtime (`stat -f '%Sm' …`), and About › Version. A mismatch means the installed app is not the build: stop and rebuild.
- The background daemon `namehold-syncd` may still be an old process from an earlier install; a case that relies on it ([J3](scenarios/shakedex/persistence-and-recovery.md#j3--the-background-daemon-follows-a-listing-while-the-app-is-closed)) first checks its start time.

## Modes

- **Mode A — the installed app** with existing regtest profiles. The maintainer must agree to it for the run (it writes to the real `~/.namehold` through the app, never by hand). Ask the maintainer for the seller profile's passphrase if you do not have it; never write it down.
- **Mode B — isolated dev build**: `HOME=$QA_DIR/qa-home pnpm tauri dev > $QA_DIR/tauri-dev.log 2>&1 &` (keep `CARGO_HOME` pointing at the real cargo home). It is ready when the log shows ``Running `…/debug/namehold-wallet` ``; `$QA_DIR/qa-home/.namehold/portfolio.db` must exist, which proves the isolated database is in use. You then create the profiles yourself ([Profiles](#profiles)). It rebuilds on any repo file change, so edit nothing while it runs. Stop it at the end with `pkill -f debug/namehold-wallet` if you started it.
- **Mode C — the browser mock** (`pnpm dev`, no Tauri, `src/lib/webqa-mock.ts`). Its profile is a mainnet one and nothing reaches a node, so it checks UI structure only; the Shakedex selling walk it supports is described in [QA_WORKFLOWS.md](../QA_WORKFLOWS.md) §J, last item. No case below needs it except [M3](scenarios/shakedex/mainnet-absence.md#m3--mainnet-only-selling-ui-in-the-browser-mock) and [M5](scenarios/shakedex/mainnet-absence.md#m5--the-mainnet-market-page-in-the-browser-mock).

Record the mode and the GUI driver (computer use, or an accessibility driver such as cua-driver) in the report.

## Regtest node and hsd wallet

```bash
cd $REPO
scripts/regtest.sh rpc getblockcount        # record; past 24 000: stop and ask
scripts/regtest.sh --with-wallet            # idempotent; node 127.0.0.1:14037, hsd wallet API 14039, API key "test"
```

- If the node runs without its wallet, the script says so: `scripts/regtest.sh stop`, then `scripts/regtest.sh --with-wallet` (a restart keeps the chain).
- Mine: `scripts/regtest.sh mine <n> [address]`. RPC: `scripts/regtest.sh rpc <method> [args]`.
- Regtest periods: auction tree interval 5, bidding 5, reveal 10; **transfer lockup 10 blocks**; renewal window 5 000 blocks; coinbase matures after 2 blocks. The block reward is 2 000 HNS halved every 2 500 blocks.

Shell helpers the cases use (`hsw-rpc` and `hsd-rpc` come with hsd):

```bash
W()    { hsw-rpc --network=regtest --api-key=test "$@"; }          # the hsd wallet "primary": CLI seller and CLI buyer
WADDR=$(W getnewaddress | tr -d '"[:space:]')
mine() { scripts/regtest.sh mine "$1" "${2:-$WADDR}"; }
info() { scripts/regtest.sh rpc getnameinfo "$1" | python3 -c 'import json,sys; i=(json.load(sys.stdin) or {}).get("info") or {}; print("state",i.get("state"),"owner",i.get("owner"),"transfer",i.get("transfer"),"renewal",i.get("renewal"))'; }
```

## The Shakedex CLI seller and buyer

`scripts/shakedex-cli-sell.sh` drives the real shakedex CLI, pinned to one commit and cloned into `$SHAKEDEX_WORK` on first use (about a minute; prefix that first call with `GIT_CONFIG_GLOBAL=/dev/null` if the clone fails). Its seller and its buyer are the node's hsd wallet. The commands a run uses are in [cli-list-a-name](procedures/cli-list-a-name.md) and [cli-fill](procedures/cli-fill.md).

## Profiles

Three regtest profiles of one recovery phrase, in these roles (the names are examples; use whatever the machine has, and say which in the report):

| Role | Example name | What it is |
|---|---|---|
| Seller | `Regtest Wallet` | recovery-phrase profile; sells, and buys from the CLI |
| Second device | `QA Restore Regtest` | the same recovery phrase imported again as a second profile; restores locks by name |
| Watch-only | `QA Watch Regtest` | watch-only profile from the seller's account xpub |

Every profile connects as **Remote Node** `http://127.0.0.1:14037`, API key `test`, with **Allow sending via remote node** ticked (Settings → Connections, then `Save settings`).

Creating them (mode B, or when a profile is missing):

1. Onboarding: network **Regtest (local testing only)**; card **Remote Node** with the URL and key above; tick **Allow sending via remote node**; **Test connection** (it uses the key just typed, before anything is saved), then **Continue**.
2. **Create a new wallet** (not watch-only, not Ledger): a secure window asks for a passphrase, then shows the recovery phrase; confirm the backup.
3. Second device: Add wallet → import the same recovery phrase as a regtest profile.
4. Watch-only: Add wallet → watch-only → import the seller's account xpub (the secure window asks "Enter the account xpub").

## Funding the app

Read the seller's **Receive Address** (`rs1…`) on the Wallet page, then `mine 12 <rs1…>` and **Sync**: the coinbases mature 2 blocks later. At heights past 20 000 a block pays under 8 HNS, so mine more if the buying cases need it (a full buying run spends about 30 HNS on purchases).

## Test names for a run

Roles used by the scenarios; the real names carry the run's `TAG`, and the report maps each role to its real name.

| Role | Name | Owner after setup | Used by |
|---|---|---|---|
| SA … SF | `qfa$TAG` … `qff$TAG` | the seller profile | selling, restore |
| X1 | `qxa$TAG` | hsd wallet, listed by the CLI, Buy Now 5 HNS; **one bid**, so its lock coin is worth 0 | buying |
| X2 | `qxb$TAG` | hsd wallet, listed by the CLI, reverse auction 10 → 5 HNS | buying |
| X3 | `qxc$TAG` | hsd wallet, listed by the CLI, Buy Now 5 HNS | buying, the race |
| X4 | `qxd$TAG` | hsd wallet, listed by the CLI, Buy Now 5 HNS (only for B13) | buying |
| X5 | `qxe$TAG` | spare CLI listing, for a repeat of a race or a node-down case | optional |

Register them all in **one** auction cycle with the hsd wallet (about 36 blocks for all, far cheaper than the script's `REGISTER=1`, which mines 110 funding blocks per name), then move the app's names to the seller:

```bash
W getbalance                                # needs ~50 HNS spendable; else mine 10, then 2 more
APP_NAMES="qfa$TAG qfb$TAG qfc$TAG qfd$TAG qfe$TAG qff$TAG"
CLI_NAMES="qxa$TAG qxb$TAG qxc$TAG qxd$TAG"
for n in $APP_NAMES $CLI_NAMES; do W sendopen $n >/dev/null; done; mine 6
for n in $APP_NAMES $CLI_NAMES; do info $n; done                  # all BIDDING, else mine 1 and look again
for n in $APP_NAMES qxa$TAG; do W sendbid $n 1 2 >/dev/null; done # one bid: X1's lock coin is worth 0
for n in qxb$TAG qxc$TAG qxd$TAG; do W sendbid $n 1 2 >/dev/null; W sendbid $n 0.5 2 >/dev/null; done
mine 6                                                            # REVEAL, else mine 1
W sendreveal >/dev/null; mine 11                                  # CLOSED, else mine 1
for n in $APP_NAMES $CLI_NAMES; do W sendupdate $n '{"records":[{"type":"TXT","txt":["shakedex-cli-sell"]}]}' >/dev/null; done; mine 1
APP=<the seller's Receive Address, rs1…>
for n in $APP_NAMES; do W sendtransfer $n $APP >/dev/null; done; mine 11
for n in $APP_NAMES; do W sendfinalize $n >/dev/null; done; mine 1
```

Then **Sync** in the seller profile: all six app names appear in Owned Names, and `info <n>` shows `transfer 0` with the owner at the app's address. If they do not, that is a finding (owned-name discovery); fall back to the in-app auction ([REGTEST_TESTING.md](../REGTEST_TESTING.md) §6, about 25 blocks for all names at once). Then list X1 (`fixed`, PRICE 5), X2 (`auction` 10 → 5), X3 (`fixed`, PRICE 5) and, only for B13, X4 with [cli-list-a-name](procedures/cli-list-a-name.md), 13 blocks each. Setup total: about 36 + 39 (+13) blocks, plus the funding blocks.

A chain reset leaves the app profiles holding rows and drafts about names that no longer exist on chain; rows that a Sync does not clear are leftovers of the reset, noted once in the report, not findings.

## A second node without a transaction index

No GUI case needs one today. The live tests that do, and how to start one on ports of its own, are in [REGTEST_TESTING.md](../REGTEST_TESTING.md) ("A node without a transaction index"). A case added for spec §5 "A node without a transaction index sees less" would point a throwaway profile at such a node, never a shared one.
