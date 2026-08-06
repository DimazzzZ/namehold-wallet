# Changelog

## [Unreleased]

### Added
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
- **MIT LICENSE** — repo now carries an explicit MIT license file.

- **`get_resource` Tauri command** — returns combined name info + DNS resource
  records from the local hsd node with graceful degradation to the explorer for
  name state. Currently backend-only (no frontend consumer after the DNS page
  removal in a later commit).

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

### Fixed
- **Sync UI view jumping** — automatic sync (every 60s) no longer expands the
  full sync status panel. Auto-sync shows "Syncing…" on the button with a
  spinner; the full progress panel only appears for manual Sync.
- **Stale-data banner suppressed when node is live** — the "Couldn't verify
  urgent auction tasks — data may be stale." banner no longer appears during
  transient query hiccups while the node is synced; it only shows when the
  node is actually offline.

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
