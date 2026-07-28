# Changelog

## [Unreleased]

### Changed
- Activity view (full table + Wallet "Recent activity" card) now shares the
  same typography and density as the Owned Names table (compact `py-1` rows,
  plain gray headers, `font-mono` names and heights). Names, txids, and
  block heights in Activity are clickable Shakeshift explorer links on
  mainnet. The in-app "manage name" modal that used to open from the
  Activity name click has been removed — use the Manage buttons on the
  Wallet Owned Names or the Auctions rows instead.
- Explorer links throughout the app now open on Shakeshift
  (https://shakeshift.com): names (auctions, owned-names, recent-tx,
  inventory, renewals), block heights (confirmation, start, renewal), txids,
  and receive-address. External-explorer UI labels are brand-neutral. The
  read API backend is unchanged (still e.hnsfans.com by default).

### Added
- **Reveal in-flight UI** — after broadcasting a reveal, the modal stays open
  and shows a pending-confirmation card (txid + copy + explorer link) instead of
  closing. The auctions row advances through `revealBroadcastPending` →
  `revealDoneWaitingForClose` → won/lost without a manual refresh (30s polling +
  draft-confirmation watcher with a proactive toast). A confirm-before-broadcast
  panel shows the bid amount and cycles substate labels (Unlocking → Signing →
  Broadcasting). Covers restored/cross-device wallets via a chain-truth fallback.
- **Block-driven stateful mock engine** — `pnpm dev` (browser) now runs a
  virtual blockchain simulator with mutable `chainHeight` and per-name auction
  records. Names advance deterministically through AVAILABLE → OPENING → BIDDING
  → REVEAL → CLOSED as blocks are mined via `__webqa_mine(n)`. The seeded
  scenario pre-loads a name in REVEAL phase ready to reveal.
- **External link opener** — `tauri-plugin-opener` enables "View on explorer"
  buttons that open the system browser (Shakeshift tx page). Falls back to
  `window.open` in the browser dev mode.

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
