# Changelog

## [Unreleased]

### Security
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
