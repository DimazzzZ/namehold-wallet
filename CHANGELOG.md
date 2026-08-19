# Changelog

## [Unreleased]

### Fixed
- **`S` (and other write-gated) keyboard shortcuts now explain themselves.**
  Pressing `S` on the Wallet page to open Send used to do nothing at all
  when the wallet couldn't send yet (read-only wallet, locked signer, or
  coins not synced) — no modal, no message. It now surfaces the same
  guidance the disabled Send button shows (e.g. "Unlock your wallet to sign
  transactions.") as a toast, so the key never silently no-ops.
- **Inline draft actions in Activity table.** Draft rows (unsigned, failed,
  signed-but-unbroadcast) now show contextual "Sign & broadcast" / "Broadcast"
  / "Retry" and "Discard" buttons directly in the row, so you can act on
  pending drafts without navigating away. The Txid cell no longer renders a
  disabled button when there's no txid — it shows a plain dash instead.
- **Actions column stays single-line.** The new Actions cell uses
  `whitespace-nowrap` so buttons never wrap to a second line.

### Added
- **Keyboard-first navigation.** A cheatsheet overlay (Shift+?) documents
  every binding, and every panel now has route-scoped shortcuts so common
  flows never need the mouse:
  - **Command palette** (⌘K / Ctrl+K) — fuzzy-searchable list of the
    navigation targets and view actions available on the current page.
    Type to filter, ↑/↓ to move, Enter to run, Esc to close. Write-only
    actions (Send, Batch Bid) are hidden on read-only wallets so you can't
    open a dead-end flow.
  - **Wallet (`/`)** — `s` open Send, `r` refresh (Sync), `u` toggle
    lock/unlock, `q` toggle the receive QR, `/` focus the name filter, and
    `j`/`k` + `Enter` to walk the Owned Names list and open the selected
    row's Name Actions modal without touching the mouse.
  - **Auctions (`/auctions`)** — `/` focus the lookup input, `b` open the
    Batch Bid modal.
  - **Activity (`/activity`)** — `/` focus the search input.
  - **Watchlist (`/watchlist`)** — `a` focus the add-name input, `e`
    export CSV.
  All action keys are suppressed while an input is focused or a modal is
  open (the palette itself layers above dialogs), so shortcuts never
  hijack typing. The cheatsheet groups bindings by category and filters
  action/list keys down to the ones that actually work on the current
  page, keeping the reference honest.
- `useDeleteTxDraft` mutation hook — lets the frontend discard a draft and
  free its reserved coins.
- `draftId` field on merged activity rows — enables the UI to target specific
  drafts for sign/broadcast/discard without a lookup.
- **Fee-rate control** — a global default fee rate in Settings > Advanced
  (`fee_rate_doos_per_kvb` setting, in doos per 1000 vbytes) plus a
  per-transaction override widget (`FeeRateOverride`) shown in every
  transaction flow: Send, Batch Renew/Reveal/Redeem/Finalize, single Bid
  (Name Actions modal), and Batch Bid. The override is a collapsible
  "Advanced" disclosure with validation, min-value clamping (1000 doos/kvB =
  1 sat/byte), and inline help. The Rust backend's `resolve_fee_rate` now
  reads the setting before falling through to `estimatesmartfee` or the
  relay-floor default. New shared library: `src/lib/feeRate.ts`
  (parseDoosPerKvb, doosPerKvbToSatsPerByte, parseFeeRateArg).
- **DEV: Simulate update flow** — a dev-only "Simulate update available"
  panel in Settings (gated behind `import.meta.env.DEV && isTauri()`) seeds
  the shared `useAppUpdate` store from the latest GitHub release (or a
  synthetic bumped version when offline) so the banner + Settings card show
  the "available" notice without auto-installing. Clicking "Install now" then
  runs a fake download loop (10 ticks × 120 ms → installed) via a `simulated`
  flag on the store, so the full update UX can be exercised without a real
  signed release. New Rust command: `fetch_latest_release_meta` (debug-gated).
- Rust integration test: `build_batch_bid_draft` rejects batches containing
  any name not in BIDDING/OPENING phase and persists nothing (all-or-nothing
  atomicity guard).

### Fixed
- **"Launch at login" no longer starts the wrong build.** If you'd ever
  enabled launch-at-login from a development build, macOS would keep
  starting that stale dev binary at login instead of the installed
  Namehold app — showing a blank window and the wrong Dock icon.
  Autostart is now registered only in release builds, and the
  "Launch at login" toggle is disabled in dev builds, so a development
  session can no longer hijack your login item. (If you hit this, delete
  `~/Library/LaunchAgents/Namehold.plist`, then re-toggle "Launch at
  login" from the installed app to re-register it correctly.)

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
