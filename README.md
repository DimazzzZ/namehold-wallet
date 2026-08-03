# Namehold — a non-custodial Handshake (HNS) wallet

Namehold is a local desktop wallet for **Handshake (HNS)**: hold HNS, manage the
names you own, run the full name-auction lifecycle, and edit on-chain DNS — all
non-custodially, with your keys encrypted on your own machine. It also includes a
guided **Move from Namebase** helper for migrating names and funds off the
custodial service.

Built with Tauri v2, React + TypeScript, Rust, and SQLite.

<p align="center">
  <img src="docs/assets/wallet.png" alt="Namehold wallet" width="700" />
</p>

> ⚠️ **Beta software.** Namehold is under active development and can make mistakes.
> Transactions and Namebase transfers are **irreversible** — always test with a
> single name or a small amount and confirm it arrives **before** sending or
> transferring everything.

## Features

### Wallet (the core)
- **Create or import** a wallet from a BIP39 mnemonic, or add a **watch-only**
  wallet from an account xpub.
- **Multiple wallets** — switch between them and delete ones you no longer need.
- **Receive** — your address with a QR code, one-click copy.
- **Per-wallet balances** — each wallet shows its own balance; values persist and
  refresh on demand (no bleed between wallets).

### Send HNS
- A **build → sign → broadcast** draft flow: preview fee/change before any key is
  touched, sign in the secure window, then broadcast.
- **Send Max** to sweep a wallet.
- **Status tracking** — sent transactions move Pending → Confirmed (with block
  height), or are flagged "Not confirmed" if they never make it on-chain.

### Names & auctions
- The complete Vickrey-auction lifecycle: **open, bid, reveal, redeem, register,
  update, transfer, finalize, cancel, renew, revoke**.
- **Phase badges + countdowns** for each owned name, a **reveal-required alert**,
  and a **"Locked in Auctions"** balance for in-flight bids.
- A typed **DNS-record editor** (TXT/A/AAAA/NS/CNAME…) for register/update, with a
  raw-JSON fallback.

### Authenticated chain reads
- A local or remote **hsrd sidecar** restores wallet history and UTXOs, reconciles
  mempool activity, supplies strict name-proof evidence, quotes fees, admits
  signed transactions, and relays them to peers through wallet RPC v1.
- Namehold retains custody: encrypted seed storage, BIP39/BIP44 derivation, and
  transaction signing remain entirely inside the wallet process.

### Move from Namebase (one feature, not the core)
- Connect with your Namebase session cookie to **list custodial domains**, see
  which are **expiring soon**, **transfer names out** to your wallet, **withdraw
  HNS**, and **compare** your inventory against what Namebase still holds.

### Portfolio (optional — enable "Advanced mode" in Settings)
- A migration-inventory workspace: **CSV import/export**, tags/filters, **batches**,
  **renewals** tracking, and per-name **migration-status** tracking.

### Auto-update (since v0.2.0)
- Namehold checks for updates automatically ~30 seconds after launch. When a
  new version is available, a banner appears at the top of the window offering
  a one-click install; you can also check manually in **Settings > Updates**.
  Update bundles are **Ed25519-signed** at release and verified against the
  embedded public key before install; unsigned or tampered bundles are rejected.

### Background sync (since v0.3.0)

- Namehold can keep your wallet data fresh even when the app is closed. A
  lightweight background daemon (`namehold-syncd`) wakes every 60 seconds and
  restores all wallet profiles from authenticated hsrd wallet RPC v1 into the
  shared SQLite database.
- Controlled by a Settings checkbox **"Sync in background"** (default ON).
- When enabled, hsrd stays running after you close the app so the daemon can
  query it; the next app launch adopts the running node (no duplicate spawned).
- Crash recovery: if the daemon dies, the app respawns it on startup.
- The daemon is **read-only** — it never signs transactions or broadcasts.

## How it works

- **hsrd supplies evidence, not custody.** Chain tip, wallet restoration, name
  proofs, mempool reconciliation, transaction admission/relay, fee quotes, and
  swap-contract activity come from authenticated wallet RPC v1.
- **hns-rs supplies canonical formats.** Transaction, covenant, script/signature
  hash, swap, and marketplace protocol types come from one pinned hns-rs revision.
- **Namehold signs.** Seed encryption, BIP39/BIP44 derivation, transaction planning,
  and local signatures stay in Namehold. The sidecar never receives private keys.
- **Secrets stay in a secure window.** Your mnemonic/passphrase is only ever typed
  into — and your backup phrase only ever shown in — a small **Rust-owned window**;
  it never passes through the web UI. At rest it's an **encrypted vault**
  (Argon2id + AES-256-GCM) inside the local database.
- **Write-capability gating.** Spend/name actions are enabled only when the signer
  is unlocked **and** the sidecar is reachable, synced, and wallet-indexed — with a
  precise reason shown when any condition isn't met.
- **Background sync daemon.** When "Sync in background" is enabled (Settings →
  Connections, default ON), a separate Rust binary syncs all profiles every 60s.
  A cross-process DB lock table (`sync_locks`) coordinates the app's manual Sync
  and the daemon via heartbeats (10s) and stale-lock takeover (30s) so they never
  write the same profile concurrently. The daemon writes its PID to
  `~/.namehold/syncd.pid` for lifecycle tracking.

## Security

For a detailed threat model, attack surfaces, and mitigations, see [SECURITY.md](./SECURITY.md).

- **Non-custodial** — your keys live on your device, encrypted; nothing is custodied.
- **Secrets never reach the web layer** — entry/display happen in the secure window,
  and signing happens in Rust.
- **Local-first** — keys and secrets stay on your device (encrypted at rest). A
  managed loopback hsrd sidecar keeps wallet restoration and chain evidence local.
  Remote sidecars require HTTPS when Authorization is configured. No telemetry.
- **Auto-lock** — the unlocked signer times out after a configurable idle period.
- **Namebase migration** — optional in-app helper for transferring domains from
  Namebase. The session cookie is encrypted at rest and never exposed to the web
  layer. See [SECURITY.md](./SECURITY.md) for the full threat model and a
  lower-risk alternative.

## Prerequisites

- [Node.js](https://nodejs.org/) 22+
- [pnpm](https://pnpm.io/) 11+ (CI pins `11.17.0` via the `packageManager`
  field; run `corepack enable` to match it locally)
- [Rust](https://www.rust-lang.org/tools/install) (stable, edition 2021)
- [hsrd](https://github.com/handshake-rs/hns-node-rs) 0.3.4+ — authenticated
  chain sidecar, built with `cargo build --release -p hns-node --bin hsrd`.

## Quick start

```bash
git clone <repo-url>
cd namehold-wallet
pnpm install
pnpm tauri dev
```

## Running the sidecar

For wallet restoration, chain evidence, and broadcast, run hsrd with wallet RPC
v1 enabled:

```bash
hsrd --network mainnet --data-dir ~/.hsrd \
  --rpc-bind 127.0.0.1:12037 \
  --rpc-authorization-header-file ~/.hsrd/namehold-wallet.authorization \
  --native-sync --p2p-discovery --wallet-index --storage-mode archive \
  --mining-engine --transaction-relay \
  --acknowledge-incomplete-consensus
```

- `--wallet-index`, `--native-sync`, and an exact Authorization-header file are
  required for the versioned wallet boundary.
- The app can manage hsrd for you — set the **data directory** and (if needed) the
  **hsrd binary path** in **Settings → Connections**, then click **Start hsrd**.
- Re-sync creates a timestamped backup before starting a fresh wallet-indexed
  data directory.

| Network | Suggested RPC port |
|---------|---------------|
| Mainnet | 12037         |
| Testnet | 13037         |
| Regtest | 14037         |

See [`docs/NODE_SETUP.md`](docs/NODE_SETUP.md) and
[`docs/REGTEST_TESTING.md`](docs/REGTEST_TESTING.md) for details.

## Move from Namebase

A guided migration helper (not the wallet's core function). In **Move from
Namebase**, paste your Namebase session cookie to connect, then:

<p align="center">
  <img src="docs/assets/namebase-migration.png" alt="Namebase migration" width="700" />
</p>

- review your custodial domains and which are **expiring soon**,
- **transfer** names out to your own wallet address,
- **withdraw HNS** to an address,
- **compare** your imported inventory against Namebase's current list.

On-chain finalization of transfers uses the same node-backed write path as the
rest of the wallet.

## Advanced: Portfolio inventory

Enable **Advanced mode** in Settings to show the **Portfolio** workspace for
tracking a larger migration.

**CSV import** columns:

```csv
Name,Staked,Category,Tags,Notes
crypto,true,Premium,"high_value,operational",High-value TLD
wallet,false,Finance,"medium_value",Finance TLD
```

- **Name** (required; leading dots stripped) · **Staked** (`true`/`1`/`yes` ⇒ staked)
  · **Category** · **Tags** (comma-separated) · **Notes**.
- Staked names are set to `do_not_touch_staked` so they're never migrated.

Per-name **migration status**: `not_started` → `namebase_transfer_requested` →
`waiting_transfer_tx` → `transfer_seen_on_chain` → `waiting_finalize` →
`finalized_owned`, plus `failed_or_stuck` and `do_not_touch_staked`.

## Build for production

```bash
pnpm tauri build
```

Output in `src-tauri/target/release/bundle/` — macOS `.app`/`.dmg`, Windows `.msi`,
Linux `.AppImage`/`.deb`. CI (PR tests), a dependency-audit gate (`cargo audit` +
`pnpm audit`), and the cross-platform release pipeline live in
[`.github/workflows`](.github/workflows).

## macOS

The macOS build is not code-signed. On first launch macOS may show:

> "Namehold" can't be opened because Apple cannot check it for malicious software.

To remove the quarantine flag, run:

```bash
xattr -cr /Applications/Namehold.app
```

Then open the app normally.

## Data location

All app data lives in one SQLite file in your home folder (pairs with hsrd's `~/.hsrd`),
on every platform:

- `~/.namehold/portfolio.db`
- `~/.namehold/syncd.pid` — background sync daemon's process ID (present only
  while the daemon is running)

It holds your wallet profiles, the encrypted vault, the local chain cache, and (if
used) the Portfolio inventory/batches/audit log.

## Tech stack

- **Tauri v2** — desktop shell
- **React 19 + TypeScript** — frontend
- **Vite** — build tool
- **TanStack Query** — async state (TanStack Table + Virtual for the Portfolio grid)
- **Zustand** — client state
- **Zod** — validation
- **SQLite (rusqlite)** — local database
- **hns-rs** — canonical Handshake transactions, covenants, scripts, swaps, and marketplace protocol
- **reqwest** — authenticated hsrd wallet RPC and optional public-data HTTP client
- **secp256k1 · bip39 · argon2 · aes-gcm · zeroize** — keys, mnemonics, vault crypto
- **Tailwind CSS** — styling
