# Changelog

## Releases

- [0.6.1](#061---2026-10-25)
- [0.6.0](#060---2026-10-23)
- [0.5.0](#050---2026-08-20)
- [0.4.1](#041---2026-08-11)
- [0.4.0](#040---2026-08-07)
- [0.3.0](#030---2026-07-29)
- [0.2.1](#021---2026-07-25)
- [0.2.0](#020)
- [0.1.0](#010---2026-06-30)

## [Unreleased]

### Added
- **ICANN collision flag.** A name that someone has also applied for as a new gTLD in ICANN's 2026 round (1,614 applications on 1,203 strings as of 29 October 2026) carries a yellow ICANN badge on the Names page and in the header of its own page; the badge's tooltip gives the applicant count and whether the string is in contention. Details opens with an "ICANN 2026 round" row naming the applicants, each a link to the application on ICANN's site, and the date of the list; it shows even for a watched name nobody has opened on-chain. The Filters popover gets an "ICANN 2026 round" switch, kept in the URL as `icann=collision`. The list of strings ships with the app, refreshed by `just icann-strings`; the app never contacts ICANN. An applicant who switched to a replacement string counts only under the string they ended on.

## [0.6.1] - 2026-10-25

> **Highlights:** wallets on different networks each get their own node, started when you switch to them; the Linux AppImage runs background sync and starts for every user of the machine; and fixes to Speed up in Activity, finalize reminders and testnet nodes.

### Added
- **A node per network, started for the wallet you switch to.** Wallets on different networks no longer share one hsd: switching to a mainnet wallet starts the mainnet node (when it uses a local node and its network starts on its own) and leaves your regtest node running, so switching back is instant. Full or SPV and "Start the node automatically" are now chosen per network, shown for the active wallet's network in Settings; networks you never set keep your current choice. Start and Stop, in Settings, the status bar and the menu-bar icon, name the network they act on, and the status shows "Starting mainnet node…" while it comes up. A node you run yourself is used but never stopped by Namehold, and its Stop is disabled with the reason. With background sync on, quitting leaves every node Namehold started running for the daemon; with it off, quitting stops them all, on macOS and Linux including one left from an earlier session (on Windows a node left running is not recognised after a restart, so it is used but not stopped).

### Fixed

**Linux**
- **AppImage: background sync runs, and the app starts for any user.** The 0.6.0 AppImage shipped the background sync daemon (`namehold-syncd`) without its executable bit, so background sync and the finalize reminders with the app closed could not run from it, and its launcher could be started only by the user who mounted the image, so it failed under firejail and in the AppImage catalog's test ("AppRun.wrapped: Permission denied"). Both files are now executable by everyone. Every build of the AppImage, on each change to its packaging and before each release, is now checked by starting its daemon and the app as another user. The `.deb`, `.rpm`, macOS and Windows builds were not affected.
- **The app starts for a second user of the same machine.** The tray icon's file went to the shared `/tmp/tray-icon` when a session has no `XDG_RUNTIME_DIR`, which the first user to run Namehold created, so for anyone else the app stopped at start with "tray icon error: Permission denied". Each user's icon now goes to their own cache directory.

**Nodes**
- **Switching between wallets of different networks no longer sends one to the other's node.** A wallet using a local node, with no node address of its own, now always reaches its own network's node (mainnet 12037, regtest 14037, testnet 13037, simnet 15037) instead of the address the node started last left in Settings, and starting a node no longer rewrites that address. A node address you set by hand, such as another machine on your network or a custom port, is kept as before.
- **A local testnet node is no longer refused as being on the wrong network.** hsd names testnet `test` in its chain info, and Namehold only knew the spelling `testnet`, so starting or adopting a testnet node failed with "node network mismatch: active profile is testnet, but the node on this port reports chain test".
- **A name the node describes in a shape the app cannot read is no longer shown as available.** When the node answered for a name with on-chain state the app could not parse, the name page treated it as never opened and offered to open an auction. The app now asks the explorer instead, as it does when the node cannot answer.

**Activity, reminders and listings**
- **A Speed up stays a Speed up in Activity once the node reports it.** The child transaction of a Speed up read "Send" as soon as its row came from the node (sent or confirmed), and showed a disabled Speed up button of its own. It now reads Speed up, linked to the transaction it speeds up, with no button. Purchases sped up showed the same.
- **A "Ready to finalize" reminder comes once, not again after a restart.** A purchase, listing or cancel whose lockup a sync briefly saw as still running (as can happen while the app syncs on launch) lost its "already reminded" mark, and the next check sent the same reminder a second time. The mark now stays until the FINALIZE is sent; a FINALIZE that fails or is dropped still reminds again, and a listing still reminds once a day. The background daemon also writes a log at `~/.namehold/syncd.log` (each sync, each reminder sent, a listing going back to Locking, errors), kept under 2 MB.
- **My listings says "Sold at a price set on another device" for a listing sold from another device before this one listed it.** A listing that another device with your recovery phrase finalized and sold, while this device still had it waiting for the lock, carried no price here, and the line was left out. It now shows whenever the wallet's coin in the sale names none of this device's prices, including when this device has none.

### CI / tooling
- Take `zod` `4.6.7` to clear GHSA-mrhx-c7xc-m3w5 (memory exhaustion validating a large array), which failed the `pnpm audit --prod` gate; no 3.x release is patched. The app parses nothing with zod at run time (its schemas give types and tests only), and the schemas needed no change.
- The Linux AppImage is built and checked on every pull request that touches its packaging or the daemon (`appimage.yml`, `scripts/check-appimage.sh`): file modes, the daemon run as another user, and the app started as another user under a virtual display. The release build runs the same check before a release is published.
- A coverage pass over the Rust crate (lines 97.1% → 98.4%), with tests that keep off the developer's real keychain and `~/.hsd`, and a live test that runs a real regtest and testnet hsd side by side (`HNS_IT_SPAWN_HSD=1`, run locally like the rest of the live suite).

## [0.6.0] - 2026-10-23

> **Highlights:** buy and sell Handshake names through Shakedex, speed up a stuck transaction, connect to a remote node or SPV from the first screen, place several bids on one name, and a long round of fixes to bids, reveals, redeems, transfers and fees.

### Added

**Shakedex: buy names**
- **Market** (sidebar, after Auctions; hotkey `4`): LearnHNS Market listings and the listings you import (file, pasted JSON or a market.learnhns.com link) in one list, each verified against your own node before you can buy. Search, filter (fixed price, price drops) and sort, with a panel for the selected listing: price or price schedule, what your node checked, and the full breakdown (price, LearnHNS fee, network fee, total) before Buy.
- The price is checked again just before sending; if it changed, nothing is sent. Names that would expire before a purchase can be finalized cannot be bought, and ones close to it are marked "Expires soon".
- **Your purchases** card shows each name bought and not yet finalized, how far its transfer lockup is, and Finalize when it is over. Owned Names follows the purchase too (Unconfirmed purchase → Awaiting finalize → Ready to finalize), and you get a notification when a name is ready. A purchase that ends without the name reads "Lost" with the reason and whether anything was paid.
- On mainnet, buying and selling stay behind the "Allow buying and selling names on mainnet through Shakedex (experimental)" setting; the Market offers to turn it on. Buying and selling need a recovery-phrase wallet and a node that can send. Keyboard: `/` search, `i` import, `j`/`k` move, `Enter` acts.

**Shakedex: sell names**
- **Sell** (in a name's actions): lock the name as a Buy Now or a reverse auction, optionally published on LearnHNS Market, then set the price or schedule at Finalize & sign. The dialog says what locking costs (no DNS changes or renewals while listed) and warns about a name that expires soon.
- **My listings** shows every listing with its state, market status and current price, and only the actions its state allows: Lower price, Cancel listing and Finalize home, Export listing file, Publish later. Each sync, in the app and in the background daemon, follows the listing on chain through Listed, Sale pending, Sold, Cancelled or Expired, keeps the LearnHNS copy at the current price, and reminds you when a step is ready.
- **Restoring a wallet finds its listings.** The first sync after a restore finds the names still in this wallet's Shakedex lock (needs a node with `--index-tx` and `--index-address`); later searches run only when you press Find my locks. Restore a lock by name and importing a saved listing file cover the rest.
- **Two devices, one listing.** A listing managed from another device of the same recovery phrase is marked "Managed from another device"; this device stops overwriting the market copy and offers Manage from this device to take it over.
- **Expiry warning.** My listings says when buyers will stop seeing a listing because its name nears expiry, and how to renew it.
- Finalize reminders now come from the background daemon, so they arrive with the app closed, and never twice.

**Wallet**
- **Speed up** a sent, unconfirmed transaction or purchase that pays too little: a child transaction spends its change and pays enough for both to reach the rate you choose (hsd has no replace-by-fee). The confirmation shows the rates and fees, and when Speed up is not possible the button says why.
- **Several bids on one name**, each with its own value and lockup; your own bids are highlighted in the bids panel, and Reveal and Redeem take all of them in one transaction.
- **Transfer several names at once:** select owned names and send them to one recipient in a single transaction.
- **Choose how to connect on first run:** pick the network, then a local full node, a remote node (Test connection checks it is reachable, synced and on that network) or SPV. Settings offers the same choices, plus "Allow sending via remote node" (off by default). Your keys never leave the device.

### Fixed

**Linux**
- The AppImage now has a working `.DirIcon` and desktop entry (category and description included), so AppImage installers and the AppImage catalog can read it.

**Balances and nodes**
- A node without the address index no longer shows the wallet as empty (balance 0, sends failing); the wallet says what the node is missing.
- Restored, imported and watch-only wallets now find coins past their first 20 addresses; the address gap limit is 1 to 1000.
- Freshly mined coins are shown as not yet spendable, with the blocks left, and are never picked for a send.
- A reclaimed bid lockup counts as spendable again.
- A testnet or regtest wallet never reads a mainnet node or explorer, and a node on another network is refused for syncing and sending, not only reads. Each wallet profile uses its own node settings.
- Starting a local node on testnet or regtest connects to the right port; a syncing node no longer looks stuck at 99.9%.
- Start hsd and Re-sync chain refuse to run without a known network instead of falling back to mainnet.

**Sending and fees**
- Fees are charged on virtual size: transactions paid up to twice the rate you chose.
- A send the node did not accept says "Not sent" instead of "sent"; a send marked dropped that is mined after all becomes confirmed; a mined send is no longer reported as not sent on a node without a transaction index. A proxy error or a wrong API key no longer fails a broadcast.
- A transaction too large to confirm is refused before signing, and a renewal, finalize or redeem the network would refuse is refused with the reason.
- A sent, dropped or failed send is never signed or sent again.
- SPV never broadcasts, and a remote node sends only with "Allow sending via remote node" on; both are enforced where the transaction is sent, not only in the UI.
- The secure confirmation window refuses a transaction whose summary it cannot read.
- The batch confirmation shows the amount it moves, and every confirmation counts all outputs of a name action.

**Bids, reveals and redeems**
- The bids panel no longer reads "No bids yet" for a name that has bids; bids from a name's earlier auction no longer mix into the current one, and a bid you just placed shows while it waits for a block.
- Lost lockups are named with their amount: an unrevealed bid reads "Lost — Not Revealed", and a lockup stranded in an auction nobody revealed in is shown instead of disappearing.
- A closed auction no longer reads "Owned" for a name that is not yours; without a node that can tell, it says to sync.
- Redeem works on a revoked name, is not offered in the last block before expiry, takes only the current auction's reveals, and is not offered for a reveal that won.
- A name whose auction lapsed can be opened again.

**Names and transfers**
- A freshly registered name can be updated, transferred and renewed again.
- REGISTER, RENEW and FINALIZE are no longer rejected by the node.
- Finalize waits out the transfer lockup; Renew and DNS updates no longer cancel a pending transfer by accident, and Cancel transfer needs a transfer to cancel.
- Update, Transfer, Renew, Revoke and Sign message need the name to be registered.
- A transaction of yours waiting for a block is shown as such ("waiting for a block"), and broadcasting your own register or transfer no longer reads as losing the name.
- Registering says DNS records are optional; renewal countdowns (claimed names included) and the expiring-soon warning are right on every network.

**App**
- A new wallet no longer inherits another wallet's unlocked signer.
- The xpub import asks for an xpub, not a recovery phrase.
- Export bid backup saves the file (readable by you only).
- A sync taken over by another process stops and says so.
- Move from Namebase explains that Namebase has shut down; importing your exported history still works.
- One name dialog instead of two; it shows only the sections the name's stage allows, disabled actions say why, and the bid form appears once.
- A setting stored out of range no longer blocks Save without saying why.
- Activity shows the name and the real amount for every name action; explorer links appear only where the explorer indexes the network.

### Changed
- Market sits right after Auctions in the sidebar, so Watchlist, Move from Namebase and Settings move to hotkeys `5`–`7`.
- Hover hints are the app's own tooltips, visible on disabled buttons too.
- Secondary buttons are white with a border, so each page has one filled main action.

### Removed
- The paid name swap: it could not be atomic. Offers recorded earlier are listed read-only under Activity › Paid swap offers, and the README and manual no longer describe it.
- The file-system plugin, which nothing used any more.

### CI / tooling
- README screenshots are generated by `just screenshots` from the app's browser mock with a showcase wallet, at every release.
- Lints for native `title` attributes and snake_case command arguments, and a test that keeps the file-system plugin out.
- Dependency updates for RUSTSEC-2026-0382, -0383 and -0386 (rustls-webpki, webpki-root-certs, tokio) and the yanked clipboard-win 5.4.1.
- `pnpm version:set` keeps the config files' formatting and bumps `e2e/package.json` with the app.

## [0.5.0] - 2026-08-20

> **Highlights:** Hardware wallet support (Ledger Nano S/S Plus/X), keyboard-first navigation with command palette, receive-address list, fee-rate control, and batch-bid operations.

### Added
- **Hardware wallet support — Ledger Nano S / S Plus / X** — connect a Ledger device and let it sign every transaction. Private keys never leave the device; you confirm each send, bid, or covenant action on-screen. Requires the official [`ledger-app-hns`](https://github.com/handshake-org/ledger-app-hns) firmware. Backend: new `providers/ledger/` module with APDU protocol, HID transport, multi-step signing, and device identity verification. Frontend: hardware wallet profile type in AddWalletForm, secure-window confirmation flow, and error mapping for disconnect/timeout scenarios. Database migration `026_ledger_hardware_profiles.sql`. See [docs/LEDGER.md](docs/LEDGER.md) for the full spec.
- **Fee-rate control** — a global default fee rate in Settings > Advanced (`fee_rate_doos_per_kvb` setting, in doos per 1000 vbytes) plus a per-transaction override widget (`FeeRateOverride`) shown in every transaction flow: Send, Batch Renew/Reveal/Redeem/Finalize, single Bid (Name Actions modal), and Batch Bid. The override is a collapsible "Advanced" disclosure with validation, min-value clamping (1000 doos/kvB = 1 sat/byte), and inline help. The Rust backend's `resolve_fee_rate` now reads the setting before falling through to `estimatesmartfee` or the relay-floor default. New shared library: `src/lib/feeRate.ts` (parseDoosPerKvb, doosPerKvbToSatsPerByte, parseFeeRateArg).
- **Batch bid operations** — select multiple names on the Auctions page and open a batch-bid modal showing the count and estimated total fee with a collapsible name list before signing + broadcasting. Reduces friction for bidding on many names at once.
- **Receive address list.** A "View all addresses" disclosure in the Receive card expands a scrollable list of every derived receive-branch address. Each row shows the derivation index, truncated address (with full-address tooltip), a used/fresh badge, the first-seen date, a copy-to-clipboard button, a QR-code toggle, and a mainnet explorer link. A "Generate new address" button at the bottom allocates the next unused receive index. Backend: new `list_receive_addresses` + `reveal_next_receive_address` Tauri commands. The "used" predicate (UTXO OR bid_commitment) is defined once (`derivation::ADDRESS_USED_PREDICATE`) and shared between address allocation and the list query.
- **Keyboard-first navigation.** A cheatsheet overlay (Shift+?) documents every binding, and every panel now has route-scoped shortcuts so common flows never need the mouse:
  - **Command palette** (⌘K / Ctrl+K) — fuzzy-searchable list of the navigation targets and view actions available on the current page. Type to filter, ↑/↓ to move, Enter to run, Esc to close. Write-only actions (Send, Batch Bid) are hidden on read-only wallets so you can't open a dead-end flow.
  - **Wallet (`/`)** — `s` open Send, `r` refresh (Sync), `u` toggle lock/unlock, `q` toggle the receive QR, `/` focus the name filter, and `j`/`k` + `Enter` to walk the Owned Names list and open the selected row's Name Actions modal without touching the mouse.
  - **Auctions (`/auctions`)** — `/` focus the lookup input, `b` open the Batch Bid modal.
  - **Activity (`/activity`)** — `/` focus the search input.
  - **Watchlist (`/watchlist`)** — `a` focus the add-name input, `e` export CSV.
  All action keys are suppressed while an input is focused or a modal is open (the palette itself layers above dialogs), so shortcuts never hijack typing. The cheatsheet groups bindings by category and filters action/list keys down to the ones that actually work on the current page, keeping the reference honest.
- **DEV: Simulate update flow** — a dev-only "Simulate update available" panel in Settings (gated behind `import.meta.env.DEV && isTauri()`) seeds the shared `useAppUpdate` store from the latest GitHub release (or a synthetic bumped version when offline) so the banner + Settings card show the "available" notice without auto-installing. Clicking "Install now" then runs a fake download loop (10 ticks × 120 ms → installed) via a `simulated` flag on the store, so the full update UX can be exercised without a real signed release. New Rust command: `fetch_latest_release_meta` (debug-gated).

### Fixed
- **Sync no longer wipes balances when the node is unreachable.** If every per-address coin query errors (node missing `--index-address`, or simply offline mid-sync), `sync_node_step` now bails early instead of marking all tracked UTXOs as spent — preventing a false "everything spent" balance wipe and cached-name loss on transient node failures.
- **"Launch at login" no longer starts the wrong build.** If you'd ever enabled launch-at-login from a development build, macOS would keep starting that stale dev binary at login instead of the installed Namehold app — showing a blank window and the wrong Dock icon. Autostart is now registered only in release builds, and the "Launch at login" toggle is disabled in dev builds, so a development session can no longer hijack your login item. (If you hit this, delete `~/Library/LaunchAgents/Namehold.plist`, then re-toggle "Launch at login" from the installed app to re-register it correctly.)
- **Activity page degrades gracefully without an address index.** `read_action_history` now swallows per-address history failures and shows an empty "No activity yet" state instead of surfacing an error toast when the local hsd node lacks `--index-tx` / `--index-address`.
- **Inline draft actions in Activity table.** Draft rows (unsigned, failed, signed-but-unbroadcast) now show contextual "Sign & broadcast" / "Broadcast" / "Retry" and "Discard" buttons directly in the row, so you can act on pending drafts without navigating away. The Txid cell no longer renders a disabled button when there's no txid — it shows a plain dash instead.
- **`S` (and other write-gated) keyboard shortcuts now explain themselves.** Pressing `S` on the Wallet page to open Send used to do nothing at all when the wallet couldn't send yet (read-only wallet, locked signer, or coins not synced) — no modal, no message. It now surfaces the same guidance the disabled Send button shows (e.g. "Unlock your wallet to sign transactions.") as a toast, so the key never silently no-ops.
- **Actions column stays single-line.** The new Actions cell uses `whitespace-nowrap` so buttons never wrap to a second line.

### CI / tooling
- **Fix flaky apt-get update timeout in CI.** The `Refresh apt package index` step was hard-failing when `apt-get update` timed out on a throttled Ubuntu mirror (observed on run 32273872139: rust-lint hit the 90s cap on all 3 retries while rust-test fetched the same 11 MB index in 54s on a healthier mirror). Made the step best-effort (no `exit 1`) so a slow mirror can't flake the job on the common cache-hit path. Per-attempt timeout raised 90s → 120s. Applied to both CI and release workflows.
- **Add libudev-dev to CI for hidapi crate.** The `hidapi` crate (required for Ledger HID transport) needs `libudev` system headers to compile on Linux. Added `libudev-dev` to the apt install step in both rust-lint and rust-test jobs, with cache version bump to invalidate stale cached packages.
- **Add PR auto-labeler workflow** (actions/labeler v7). New `.github/workflows/labeler.yml` + `.github/labeler.yml` ruleset automatically labels PRs by path patterns (rust, frontend, ci, docs, tests, etc.) so reviewers can filter and triage at a glance. Uses actions/labeler v7 (Node.js 24 native, no deprecation warnings on GitHub-hosted runners).
- **Ledger device simulator for testing.** A `mock-ledger` Cargo feature plus the `NAMEHOLD_LEDGER_SIM` env var swap in an in-process simulated HID transport (`src-tauri/src/providers/ledger/simulated_hid.rs`) that scripts reject, timeout, wrong-app, and disconnect scenarios — so Ledger error paths can be exercised in CI and locally without physical hardware.
- **Comprehensive test coverage improvements (PR #50).** Improved test coverage for v0.5.0 across both stacks: (1) **RPC trait injection** — introduced an `async_trait` `NodeRpc` trait as an injection seam, letting us test soft-degrade paths, error branches, and phase-dependent decisions with plain in-memory mocks instead of requiring a live hsd node or HTTP mock server. (2) **Draft-command extraction** — extracted all 9 `build_*_draft` Tauri commands into pure sync `_inner` functions testable without a Tauri runtime, each taking `&Connection` + pre-fetched inputs. This makes the core transaction-building logic directly testable with an in-memory SQLite DB — sub-millisecond per test. (3) **Test suite expansion** — 54 direct unit tests for the 9 draft commands, 78 additional pure-logic and mock-based tests for name-action capabilities, fee-rate, and context helpers, plus 5 new frontend test suites (Watchlist, WalletView action-bus, Settings fee-rate, AuctionsView batch-bid, error_tests wire-up). Total: **1405 Rust tests** (up from 1288) + **745 frontend tests** across 73 files, with **81.28% line coverage** on the Rust backend.

## [0.4.1] - 2026-08-11

A polish release focused on the tray, the auto-updater, and getting
launch-at-login working on macOS.

### Added
- **Cleaner menu-bar mode on macOS.** Close Namehold to the tray and the
  Dock icon disappears — you get a proper menu-bar-only experience while
  the app keeps running in the background. Bring the window back and the
  Dock icon returns. Clicking the Dock icon (or picking Namehold from the
  app switcher) now reliably reopens the window, and Cmd+Tab still works
  the whole time.
- **First-time "still running" hint.** The first time you close Namehold
  to the tray, a native notification lets you know the app is still alive
  in the menu bar and how to get it back. Shown once, then never again.

### Fixed
- **Tray now shows the right node status.** If hsd was already running
  when Namehold launched — say, a previous background-sync session left
  it up, or you started it yourself — the tray used to be stuck on
  "Node: Stopped / Start Node" even though the node was clearly working.
  Clicking "Start Node" would then try to launch a duplicate. The tray
  now watches the node's actual health and stays in sync with reality,
  even when the main window is closed.
- **"Launch at login" actually works now.** Toggling it in Settings used
  to fail with a permissions error. Fixed — the app will now correctly
  start with your Mac / Windows / Linux session when you enable it.
- **Nicer update flow.** The "What's new?" link in Settings now opens the
  same release-notes modal as the top banner (instead of dumping the
  notes inline). Links inside release notes — including relative ones
  like `docs/RECOVER_LOST_BIDS.md` — open in your browser correctly.
  Every button, link, and close-icon in the update flow shows a pointer
  cursor. And once an update is installed, "Restart now" can no longer
  be dismissed away by accident — restart is the only thing left to do.

### Under the hood
- Faster release builds in CI, and fixes to the macOS universal build
  and Windows installer so every release actually makes it out the door.
- Windows build fix for a type-comparison error that was breaking CI.

## [0.4.0] - 2026-08-07

### Added
- **System tray / menu-bar presence** — Namehold now lives in the system tray
  so it keeps running (local hsd node + background sync daemon alive) when the
  main window is closed. The tray menu offers **Open Namehold**, a live node
  status label with a **Start/Stop node** toggle, a **Sync in background**
  checkbox, and **Quit**. A new **System Tray** section in Settings adds two
  toggles: **Close to tray** (default ON — closing the window hides it instead
  of quitting; click the tray icon or use Open to restore) and **Launch at
  login** (registers the app to auto-start, via `tauri-plugin-autostart`:
  LaunchAgent on macOS, Run key on Windows, `.desktop` on Linux). The tray
  icon reflects node state with three variants (normal / syncing / stopped),
  rendered as a macOS template image so it adapts to light/dark menu bars. A
  3-second reconciliation ticker keeps the tray in sync with changes made
  outside it (frontend actions, hsd autostart, sync transitions). New Tauri
  commands: `is_close_to_tray_enabled`, `set_close_to_tray_enabled`. New
  settings keys: `close_to_tray`, `launch_at_login`.

- **Recover lost bids from any hsd wallet** — if you reinstall, seed-restore,
  or import a bid from another hsd-compatible wallet, Namehold can recover the
  bid value without you remembering the exact amount. The Name Actions modal
  shows a **Recover bid** panel during the REVEAL phase with two options:
  enter the amount if you remember it, or click **Auto-recover (brute-force)**
  to sweep candidate values. Recovery uses only the account xpub (public) —
  never needs your passphrase — and works because the nonce derivation is the
  hsd standard, not Namehold-specific. Typical bids under 100 HNS recover in
  seconds. See [docs/RECOVER_LOST_BIDS.md](docs/RECOVER_LOST_BIDS.md) for the
  full guide.

- **Background HSD Sync Daemon** — a separate Rust binary (`namehold-syncd`) that
  syncs wallet profiles (UTXOs, name states, transactions) from the local hsd node
  every 60 seconds, even when the app is closed. Controlled by a Settings checkbox
  **"Sync in background"** (default ON). When enabled, hsd stays running after app
  exit; the next launch adopts it. A cross-process DB lock table (`sync_locks`)
  coordinates the app's manual Sync and the daemon via heartbeats (10s) and
  stale-lock takeover (30s) to prevent concurrent writes. Crash recovery: the app
  respawns the daemon on startup if the toggle is ON and the daemon is dead.
  Bundled as a Tauri sidecar (externalBin).
- **New DB migration `021_sync_locks.sql`** — creates the `sync_locks` table for
  cross-process sync coordination.
- **SPV (Simplified Payment Verification) mode** — an opt-in lightweight alternative
  to the full node mode. SPV downloads only block headers (~几十MB vs ~15GB),
  enabling fast first launch and minimal disk usage. Balance and name data come from
  the explorer; sending is blocked in SPV mode (read-only). Controlled via a
  **"Node mode"** dropdown in Settings → Connections (default: Full node).
- **Explorer failover** — all explorer HTTP requests now support automatic failover
  to a configurable fallback URL. Configured via "Explorer fallback URL" in Settings.
- **SPV UI indicators** — StatusStrip shows "Explorer (SPV)" when in SPV mode to
  explain why reads come from the explorer.
- **SPV-aware write capability** — SPV mode shows a clear "SPV mode cannot send
  transactions" message instead of the generic "node not address-indexed" error.
- **TLD Management P1 — Batch operations** — renew, reveal, redeem, or finalize
  multiple names in one transaction. Multi-select checkboxes on the Owned Names
  table with a batch action bar ("Renew Selected" / "Reveal Selected" /
  "Redeem Selected" / "Finalize Selected"). Each action opens a
  `BatchConfirmModal` showing the count and estimated fee with a collapsible
  name list before signing + broadcasting.
- **TLD Management P2 — Watchlist** — track names you don't own for monitoring.
  New "Watchlist" page in the sidebar with add/remove, tags (comma-separated),
  bulk state fetch via `get_watchlist_status`, CSV import/export (`name,tags,
  notes,added_at,state,expiry`), and an "Add to Watchlist" toggle in both
  `NameActionsModal` and `NameInfoModal`. Database migrations
  `022_watchlist.sql` (base table) and `024_watchlist_tags.sql` (adds `tags`
  column).
- **TLD Management P3 — Atomic paid name swaps** — atomic finalize-with-payment
  covenant: the buyer finalizes a TRANSFER and pays the seller in the same
  transaction, so no party can renege after the lockup expires. Buyer side:
  "Buy with payment" button on names in TRANSFER state. Seller side: "Sell
  with payment" flow with saved offer tracking (`023_paid_swap_offers.sql`)
  and a verify-only `claim_paid_transfer` command that inspects the
  broadcast tx before marking an offer paid.

### Changed
- **Removed the Portfolio page and the "Show Portfolio in the sidebar"
  setting.** The hidden `advanced_mode`-gated Portfolio workspace (Inventory,
  Batches, Renewals, DNS tabs) and its `/portfolio` route have been removed
  entirely, along with the `advanced_mode` setting key. The sidebar no longer
  has any advanced-mode gating.

- **MIT LICENSE** — repo now carries an explicit MIT license file.

- **`get_resource` Tauri command** — returns combined name info + DNS resource
  records from the local hsd node with graceful degradation to the explorer for
  name state. Currently backend-only (no frontend consumer after the DNS page
  removal in a later commit).

- **Watchlist v2 — richer columns + background alerts.** The Watchlist table
  now shows three additional columns pulled from the live name info: a
  **Countdown** to the next phase transition (e.g. "Bidding closes in 42
  blocks (~7h)"), the **Highest bid** so far, and **Expires** (days-until-
  expire, colour-graded like the Renewals view). Names owned by the active
  wallet profile get an inline **Owned** badge next to the name. Combined
  with a new opt-in **Watchlist notifications** section in Settings, the
  background sync daemon (`namehold-syncd`) polls each watched name every
  60s, diffs against the last-seen snapshot in a new `watched_name_states`
  cache table, and fires OS notifications on: entry into **BIDDING**, a
  previously CLOSED name becoming **available again**, **bidding-soon** lead
  time (default 144 blocks / ~1 day), and a configurable **global
  highest-bid threshold** crossing (in HNS). Adaptive polling skips names
  whose next transition is > 300 blocks out when the state was refreshed
  within the last 5 minutes. The daemon is the sole notifier — the in-app
  scanner still owns reveal/renewal deadlines for names you've bid on or own,
  so there's no double-fire. Alerts fire even when the Namehold app is closed
  (as long as background sync is enabled). New Tauri command:
  `get_watched_states` (read-only, hydrates the columns without RPC on first
  page open — currently unwired on the frontend; columns hydrate via
  `read_name_info`). New settings keys: `watchlist_notify_enabled`,
  `watchlist_notify_bidding_soon_lead_blocks`,
  `watchlist_notify_highest_bid_threshold_hns`. New DB migration:
  `025_watched_name_states.sql`. New crate: `notify-rust` (cross-platform
  OS notifications, no Tauri AppHandle required).

- **Unified input/button sizing.** `Input` and `Select` now accept an
  `inputSize` prop (`"sm" | "md"`, default `"md"`) that shares the exact
  padding + text-size tokens with `Button`'s `size` prop, so a control and the
  button beside it render at identical heights. All `Button` variants now carry
  a (transparent) 1px border so bordered inputs and borderless buttons line up
  to the pixel. Replaced ad-hoc raw `<input>`/`<select>` elements across the
  Auctions, Activity, Namebase dashboard/import, wallet name filter, DNS
  records editor, and the DataTable search box with the shared `Input`/
  `Select` components. Fixes the visible ~10px height gap between the
  add-name/look-up inputs and their adjacent buttons.

- **Inline node lifecycle actions.** The StatusStrip node pill now opens a
  popover menu with **Start node** / **Stop node** / **Re-sync chain** actions
  (plus an "Open Settings" escape hatch), replacing the old Settings link. The
  WalletView "needs node sync" callout has an inline **Start node** button
  with an "Open Settings" fallback on failure. The update-installed banner
  shows a **Relaunch now** button instead of directing users to Settings.
  New reusable `Popover` component (`src/components/ui/Popover.tsx`).

### Fixed
- **macOS notification sender identity** — OS notifications from the
  background sync daemon (`namehold-syncd`) now attribute to **Namehold**
  instead of "Terminal" / Finder / a generic sender. Because the daemon runs
  unbundled (no Tauri `AppHandle`), it now claims the Namehold bundle ID via a
  `Once`-guarded `ensure_notify_identity()` before emitting; the Tauri app
  pre-empts the same identity before the notification plugin initializes. A
  new **Debug Notifications** panel in Settings (debug builds only) fires each
  notification path on demand for verification, backed by the
  `#[cfg(debug_assertions)]` `simulate_notification` Tauri command.
- **TldInventory bulk Transfer/Finalize** — bulk actions on N selected names
  now actually operate on all N, not just the first. Transfer loops N single
  transactions (per-name recipient safety); Finalize uses the batch draft
  command for a single atomic transaction. (Historical: the Portfolio/
  TldInventory workspace this fixed is removed in this same release.)
- **Layout badge color inversion** — the wallet capability badge now renders
  CAN SEND in green (safe) and READ-ONLY in neutral gray, matching intuitive
  traffic-light semantics (previously CAN SEND was red).
- **Sync UI view jumping** — automatic sync (every 60s) no longer expands the
  full sync status panel. Auto-sync shows "Syncing…" on the button with a
  spinner; the full progress panel only appears for manual Sync.
- **Stale-data banner suppressed when node is live** — the "Couldn't verify
  urgent auction tasks — data may be stale." banner no longer appears during
  transient query hiccups while the node is synced; it only shows when the
  node is actually offline.
- **Unicode/IDN name lookup** — the "Get a TLD" input now accepts Unicode
  characters (e.g. `сбер`, `münchen`) and encodes them to ACE (Punycode) at
  lookup time, instead of silently stripping non-ASCII input. Added `tr46`
  UTS-46 processing library and `src/lib/idnEncode.ts` module.

### Removed
- **Dead migration UI** — `SyncVerification` and `MigrationAssistant`
  components (plus their test) and the `compare_inventory_with_provider`
  backend command have been removed. These were scaffolding for a one-time
  Namebase migration flow that is no longer needed.

### CI / tooling
- **sccache + mold linker + debuginfo thinning** — CI cold-compile speedup
  via distributed compilation caching (`sccache`), the `mold` linker for
  faster linking, and stripped debuginfo in CI builds.
- **Parallel rust-lint + rust-test jobs, nextest adoption** — lint and test
  run concurrently; `cargo-nextest` replaces `cargo test` for per-test
  parallelism and structured output.
- **Fast/full lane split** — PR pushes run a fast lane (subset of tests);
  pushes to `main` run the full suite. Configured via `nextest.toml`.
- **sccache resilience** — graceful degradation when the GHA cache backend
  is unreachable; CI continues uncached rather than failing.
- **Argon2id test-mode cost reduction** — `cfg(test)` drops KDF from 256 MiB
  to 8 MiB, cutting vault-related tests from ~50s to <0.25s each.
- **Per-process temp DB paths** — fixes nextest parallelism flakes where
  concurrent test processes collided on the same SQLite file.

## [0.3.0] - 2026-07-29

### Changed
- **Unified table design** — all tables (Owned Names, Activity, Auctions, Renewals, Batches, DNS records, TLD inventory, Namebase dashboard) now share compact rows, consistent typography, and monospace values. Activity gained a dedicated Block column.
- **Shakeshift explorer** — all explorer links (names, txids, addresses, block heights) now open on Shakeshift (https://shakeshift.com) instead of HNSFans.
- **Release notes from CHANGELOG** — in-app update banner and GitHub releases now show the CHANGELOG entry instead of auto-generated PR titles.

### Added
- **About page** — reachable from the ℹ️ icon next to the version number. Shows logo, version, description, and a link to report issues or request features on GitHub.
- **Branded startup spinner** — animated spinner with app name instead of a blank white screen on launch.
- **Inline "Unlock" buttons** — locked-wallet notices (name actions, send, bid, finalize) now have an Unlock button so you can unlock in place without navigating away.
- **"What's new?" release notes** — the update banner shows a "What's new?" button that opens the release notes in a formatted modal.
- **Unicode-aware name search** — search by the Unicode form (`.münchen`) or the punycode form (`xn--…`) and find the same name.
- **Reveal in-flight UI** — after broadcasting a reveal, the modal shows a pending-confirmation card with txid and explorer link. Auctions rows update automatically without manual refresh.
- **External link opener** — "View on explorer" buttons open the system browser directly.

## [0.2.1] - 2026-07-25

### Changed
- **Always prefer freshest node data** — a background auto-sync now refreshes
  cached data every 60s while the local node is live and synced (kicked on the
  explorer→local edge and on mount if already live), reusing the idempotent
  `start_full_sync` and skipping while a run is in flight. Balance queries are
  no longer sticky: dropped `staleTime: Infinity`/`gcTime: Infinity`/
  `refetchOnMount: false` in favour of a 15s `staleTime` plus a node-gated 20s
  `refetchInterval` (per-profile query keys preserved, no cross-wallet bleed).
  The DNS editor in the Name Actions modal now seeds and enables UPDATE only
  from a guaranteed-fresh on-chain read (`recordsFresh` gate), so a stale base
  can never overwrite the resource.

### Security
- Encrypt the Namebase session cookie at rest under an OS-keyring-held DEK
  (AES-256-GCM). The cookie is stored as a hex-encoded blob in the new
  `namebase_cookie_v1` setting; the plaintext `namebase_cookie` setting is
  blanked on migration and on disconnect (defense in depth). Existing users'
  plaintext cookies are migrated transparently on first read.
- Add `SECURITY.md` at the repo root documenting the full threat model, per
  attack-surface mitigations, residual risks (honest disclosure), the
  lower-risk manual-transfer alternative, and a reference table mapping each
  concern to the enforcing code + tests.
- Redact sensitive settings (`namebase_cookie`, `node_rpc_api_key`,
  `hsd_api_key`) from `get_settings`; the renderer now sees only
  `__has_<key>` presence markers, never the raw value.
- Enforce a host allowlist on the Namebase API base URL, require HTTPS for
  the real Namebase host (no cleartext), and treat the `namebase_base_url`
  setting as a debug-only test seam — it is ignored in release builds.
- Deny renderer writes to security-critical settings (`namebase_base_url`,
  `namebase_cookie`) via `update_setting`.
- Require explicit user confirmation in the Rust-owned secure window before
  signing any draft (`sign_tx_draft`). The confirmation window shows the
  action, recipient, amount, fee, txid, and any warnings so a compromised
  main webview cannot swap details silently.
- Redact sensitive values in `audit_log`; `get_audit_log` also re-redacts
  legacy plaintext rows on read (defense in depth).
- Refuse to send an RPC api-key over plaintext HTTP to a non-loopback host;
  `NodeRpcClient::new` blanks the key defensively when misused.
- Ship a restrictive Content Security Policy for the Tauri webviews.

### CI / tooling
- Add a CI dependency-audit gate that runs `cargo audit --deny warnings` and
  `pnpm audit --audit-level moderate --prod` on every PR, surfacing
  newly-disclosed advisories in the dependency graph.
- Suppress reviewed, not-applicable/unavoidable advisories with justification:
  frontend via `auditConfig.ignoreGhsas` in `pnpm-workspace.yaml`, backend via
  `[advisories] ignore` in `src-tauri/.cargo/audit.toml`. Each entry is
  documented in `SECURITY.md`; a new advisory not on the list still fails CI.
- Bump `anyhow` to `1.0.104` to clear RUSTSEC-2026-0190 (unsound
  `Error::downcast_mut`) rather than suppress it.
- Pin pnpm to `11.17.0` across CI (`pnpm/action-setup`) and add a
  `packageManager` field so local (Corepack) and CI use the same version.
  Audit config moved out of the (now-ignored) `package.json` `pnpm` field into
  `pnpm-workspace.yaml`, its home under pnpm 11.
- Add a `lint:secure-imports` check ensuring nothing under `src/secure/**`
  imports from the rest of `src/`, keeping the secure-window bundle isolated.
- Harden `.gitignore` with explicit secret patterns (`*.pem`, `*.key`,
  `id_rsa`, `secrets.*`, `credentials.*`).

### Fixed
- README and USER_MANUAL privacy claim: the app is local-first, not
  local-only. The docs now spell out that the HNSFans explorer sees wallet
  addresses and tracked names by default and how to run a local hsd node
  for fully local lookups.

## [0.2.0]

### Added
- **Node-only reads + chain scanner** — owned names, balances, and per-name bid
  history read directly from the local hsd node when synced, eliminating explorer
  dependency for synced wallets. A background chain scanner indexes BID/REVEAL
  outpoints for honest bid display.
- **DNS record prefill** — the Manage DNS editor in the Name Actions modal now
  prefills existing on-chain records (`getnameresource`) so you can edit rather
  than re-enter from scratch.
- **Autostart hsd** — the app starts hsd on launch by default (toggleable in
  Settings → Connections). If hsd is already running, it adopts the existing node.
- **Message signing** — sign an arbitrary message with the wallet key that owns a
  name (proves name ownership off-chain).
- **Richer DNS editor** — real hsd record types (DS, GLUE4/6, SYNTH4/6) with
  multi-field editing; raw-JSON advanced toggle.
- **Owned-names filter** — substring search for the Owned Names list.
- **Per-bid detail in auctions** — `NameBidsPanel` shows individual bids with
  lockup/revealed values, marks your own bids, and computes an honest "highest"
  (only revealed values count).
- **Active Auctions view** — names with an open auction position merged into a
  live-phase list; pending-OPEN surfacing; double-open guard.
- **Auto-update** — the app checks GitHub Releases ~30s after launch (and on
  demand from Settings → Updates), then downloads and installs signed updates
  in place. Update bundles are Ed25519-signed at release time and verified
  against the embedded public key before install. See `docs/RELEASING.md`.

### Changed
- **WalletView density polish** — CopyField, Disclosure, and truncateMiddle
  primitives; xpub collapsed by default; balance cards consolidated.

### Fixed
- Recent transactions Amount now shows net cost (bid value), not the name's
  total locked value.
- Bid commitments now persist `bid_txid`/`reveal_txid` at build time (+ backfill
  for pre-fix commitments) so own-bid marking is reliable.

## [0.1.0] - 2026-06-30

### Added
- Non-custodial wallet: HD key derivation (BIP39/BIP32), transaction building, signing, address generation
- hsd chain backend: direct RPC to hsd node for balance, names, transactions, and mempool
- Name operations: register, transfer, renew, update, redeem, finalize
- Auction flow: bidding, revealing, and domain lifecycle tracking
- Namebase integration: import domains from Namebase, bulk transfers, renewal calendar
- hsd node control: start/stop/restart, one-click re-sync, index-setup adaptation
- Multi-provider read architecture with advanced and onboarding flows
- App shell with navigation, settings, and wallet lifecycle management
- Transaction confirmation tracking and send-max support
- Domain expiry monitoring and renewal reminders
- QR code display for receiving addresses
- CI and release workflows for automated cross-platform builds
