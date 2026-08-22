# Namehold — User Manual

Your local desktop wallet for **Handshake (HNS)**: hold HNS, run the full name
auction lifecycle, manage the names you own, and edit on-chain DNS — all
non-custodially, with your keys encrypted on your own machine.

> **Beta software.** Transactions and Namebase transfers are irreversible. Always
> test with a single name or a small amount before moving everything.

---

## Contents

1. [What Namehold is](#1-what-namehold-is)
2. [Install and first run](#2-install-and-first-run)
3. [Sidebar and header](#3-sidebar-and-header)
4. [Wallet page](#4-wallet-page)
5. [Signer: lock and unlock](#5-signer-lock-and-unlock)
6. [Write capability (when Send and name actions are enabled)](#6-write-capability)
7. [Send HNS](#7-send-hns)
8. [Auctions](#8-auctions)
9. [DNS records editor](#9-dns-records-editor)
10. [Managing names you own](#10-managing-names-you-own)
11. [Node control](#11-node-control)
12. [Move from Namebase](#12-move-from-namebase)
13. [System Tray](#13-system-tray)
14. [Data location and macOS quarantine](#14-data-location-and-macos-quarantine)
15. [Security](#15-security)
16. [Troubleshooting](#16-troubleshooting)
17. [Auto-update](#17-auto-update)

---

## 1. What Namehold is

Namehold is a **non-custodial** Handshake wallet. It holds your keys locally in
an encrypted vault (Argon2id + AES-256-GCM), signs transactions on your device,
and never sends your seed anywhere. There is **no external wallet service** — the
wallet talks directly to the Handshake network:

- **Reads (balances, owned names, name info): no node required.** Data comes from
  the HNSFans explorer by default. When your local hsd is synced, the wallet
  automatically switches to node-authoritative reads (faster, more reliable).
- **Writes (Send HNS, name actions): local hsd required.** Broadcasting a
  Handshake transaction and finding your unspent coins needs a local
  **address-indexed** hsd node — no hosted provider offers that today.

Your **secrets never touch the web UI.** Passphrases and recovery phrases are
entered and displayed only inside a small **Rust-owned secure window**.

---

## 2. Install and first run

### Prerequisites

- The Namehold desktop app (macOS `.dmg`, Windows `.msi`, or Linux `.AppImage`/`.deb`).
- To **send HNS or perform name actions**, you also need [hsd](https://github.com/handshake-org/hsd)
  — the app can start it for you (see [Node control](#11-node-control)). Reads
  work without hsd.

### First launch — Onboarding

On first launch the **Welcome to Namehold** screen opens. Pick one of three flows,
enter a **Wallet Name** and pick a **Network** (Mainnet / Testnet / Regtest), then
click the corresponding button:

| Flow | Button | What it does |
|------|--------|--------------|
| **Import your wallet** (recommended for existing users) | "Import in secure window" | Opens the secure window; paste your 12/24-word phrase + an optional BIP-39 passphrase. |
| **Watch-only (read-only)** | "Add watch-only wallet" | Adds a wallet from an account xpub. Cannot sign or send. |
| **Create a new wallet** | "Create in secure window" | Opens the secure window; sets a passphrase and displays your recovery phrase for backup. |

The **recovery phrase is only ever shown in the secure window** — the React UI
never sees it. Confirm the backup before continuing.

### Adding more wallets later

From the account bar at the top of the Wallet page, use **Add wallet** and
**Manage wallets** to switch between wallets, reveal a wallet's phrase in the
secure window, or delete one you no longer need.

### Background sync (enabled by default)

Out of the box, **Sync in background** is enabled (Settings → Connections). This
means hsd stays running after you close the app, and a lightweight background
daemon keeps your wallet data fresh every 60 seconds. If you'd rather sync only
while the app is open — and have hsd stop when you close it — uncheck **Settings →
Connections → "Sync in background"**. See [Section 11](#11-node-control) for
details.

---

## 3. Sidebar and header

The sidebar has four top-level sections:

| Section | Purpose |
|---------|---------|
| **Wallet** | Balance, receive, send, recent transactions, owned names. Default page. |
| **Auctions** | Look up a name, place bids, reveal, register, see active auctions. |
| **Move from Namebase** | Guided migration off the custodial Namebase service. |
| **Settings** | Connections, node control, backups, notifications, advanced options. |

The header shows two badges:

- **Network** chip (e.g. `mainnet`, `regtest`).
- **CAN SEND** (green) / **READ-ONLY** (grey) — the current write capability
  (see [section 6](#6-write-capability)). This is a status indicator, **not**
  a toggle. There is no "Write Mode" switch.

---

## 4. Wallet page

### Balance card

A single card shows your spendable balance in HNS as the hero number, with:

- **Confirmed / Unconfirmed** — coins that are on-chain vs. still in mempool.
- **Locked in Auctions** — total HNS locked in active bids (unspendable until
  the auction ends and coins are released).
- **Name Value** — total value of coins tied up in name covenants.

### Receive and share

A "Receive & share" card holds your receive address with a **Copy** button and a
network badge (`hs1…` mainnet, `ts1…` testnet, `rs1…` regtest). A **Show QR** /
**Hide QR** toggle renders a QR code (off by default).

Below that, a collapsible disclosure — **Show account public key (xpub)** —
reveals the account xpub. This is only useful if you're moving names off
Namebase (Namebase uses it to compute your Handshake addresses). Otherwise
leave it closed.

### Owned Names

The Owned Names table lists names your wallet controls, with a phase badge
(`OPENING`, `BIDDING`, `REVEAL`, `CLOSED`, etc.) and a **substring filter** at
the top. Click any row to open the Name Actions modal.

### Recent transactions

Send/receive history from the local cache, with block height and confirmation
status. Auction-related entries (BID, REVEAL, REGISTER, TRANSFER, FINALIZE)
show the net cost, not the full lockup.

### Urgency alerts

Yellow/red alert banners appear when a name needs your attention:

- **Reveal alert** — bids you haven't revealed yet, with a countdown.
- **Register alert** — you won an auction; register the name now.
- **Redeem alert** — you lost a bid; redeem your lockup.
- **Expiring alert** — names approaching their renewal deadline.

---

## 5. Signer: lock and unlock

Your **signer** is the in-memory decrypted key material used to sign
transactions. It is **locked by default** and unlocks only when you enter the
wallet passphrase.

- **Unlock**: click **Unlock** on the account bar. If the wallet has a
  passphrase, the secure window opens for entry. If not, it unlocks directly.
- **Lock**: click **Lock** to zero the signer immediately.
- **Auto-lock**: after a configurable idle timeout (Settings → Advanced → "Signer
  session timeout", default 900 s), the signer auto-locks.

The passphrase is **never** stored on disk — you re-enter it each session (or
leave it blank at creation for wallets with no passphrase).

---

## 6. Write capability

"Write capability" is the app's honest answer to "can I send or do name actions
right now?" It's shown in three places: the header **CAN SEND / READ-ONLY**
badge, next to the **Send** button, and inside the Name Actions modal's red
"blocked" alert.

You can write when **all** of these hold:

1. **Signer unlockable** — a wallet is loaded and either unlocked or has a
   passphrase you can enter.
2. **Node reachable** — local hsd RPC responds.
3. **Node synced** — hsd's `verification_progress` is ≥ 99.99% (or blocks meet
   headers on a network without a progress value, e.g. regtest with a single
   miner).
4. **Address-indexed** — hsd was started with `--index-address` (required to
   discover your unspent coins).

When any condition fails, the reason is shown in plain text (e.g. "Node not
synced (12%)", "hsd not address-indexed — re-sync required", "Signer locked").

---

## 7. Send HNS

On the Wallet page, click **Send HNS**. The Send dialog has:

1. **Address** — the recipient's Handshake address. Format validated on the fly
   (`hs1…` mainnet, `ts1…` testnet, `rs1…` regtest).
2. **Amount** in HNS, with a **Max** button to sweep the wallet.
3. **Review** button — builds a draft (`build_send_draft`) and shows Amount,
   Fee, Change, Inputs, and destination. **No key is touched yet.**
4. **Sign & Broadcast** — unlocks the signer (secure window if locked), signs
   the draft, and broadcasts it.

If the broadcast fails, the dialog stays open with a persistent "Not sent"
error so you don't lose your inputs. On success, the transaction shows up under
Recent transactions and transitions Pending → Confirmed as blocks arrive.

---

## 8. Auctions

Handshake name auctions are Vickrey-style sealed-bid auctions. Every unclaimed
name goes through four phases:

| Phase | Duration (mainnet) | What happens |
|-------|--------------------|--------------|
| **Opening** | ~1 day (720 blocks) | Name enters the auction. No bids accepted yet. |
| **Bidding** | ~5 days (1 440 blocks) | Anyone can place sealed bids. Bid + lockup can differ to obscure your real bid. |
| **Reveal** | ~10 days (1 440 blocks) | Bids revealed. **Fail to reveal and you lose your lockup.** |
| **Closed** | — | Highest bidder pays the **second-highest** bid and can register the name. |

### Looking up a name

1. Click **Auctions** in the sidebar.
2. Type the name (without leading dot) and click **Look up** (or press Enter).
3. The **Name Actions modal** opens and fetches the current state.
4. Follow the guided action at the top:

| Current state | Guided action | What you provide |
|---------------|---------------|------------------|
| Available / Opening | **Open Auction** | Nothing — just confirm. |
| Bidding | **Place Bid** | Bid (HNS) and Lockup (HNS). |
| Reveal | **Reveal Bid** | Nothing — just confirm. |
| Closed (you won) | **Register** | Optional DNS records. |
| Closed (you lost) | **Redeem** | Nothing — reclaim your lockup. |

Each step builds a draft, unlocks the signer if needed (secure window), signs,
and broadcasts. The transaction ID and new phase appear on the next refresh.

### Bid vs. lockup

- **Bid** — the actual value you're willing to pay. If you win, you pay the
  second-highest bid; if you lose you get everything back.
- **Lockup** — the total locked while bidding. `lockup ≥ bid`. Anything above
  your bid is a decoy and is refunded after reveal.

Both are entered in HNS.

### Active Auctions

The Auctions page shows all names you currently have positions in — pending
OPEN, open bids, reveals owed — merged into a single list with live phase.
The **Locked in Auctions** balance on the Wallet page mirrors the HNS tied up
here.

### Reveal alert

A yellow banner appears on the Wallet page any time you have bids in the
Reveal phase, with a countdown. **If you don't reveal, you lose your lockup.**

### Recovering a lost bid commitment

Handshake bids need a local "commitment" row (nonce + true value) to reveal.
Namehold stores this automatically when you bid, but if you lose the file
(reinstall without a backup, seed-restore into a fresh install, or import a
bid you made in another hsd-compatible wallet like Bob), the Name Actions
modal shows a **Recover bid** panel during the REVEAL phase.

You have two options:

1. **Enter the amount** — if you remember what you bid, type the exact HNS
   value and click **Recover bid**. Reconstruction is instant.
2. **Auto-recover** — if you don't remember, click **Auto-recover
   (brute-force)**. Namehold tries "round" values first (whole HNS, 0.1, 0.01
   increments) which almost always wins, then falls back to a full integer
   sweep up to the coin's lockup value. Typical bids under 100 HNS finish in
   seconds. Very large lockups (>1000 HNS) return an error asking for the
   known value instead.

Recovery uses only the account xpub (public) — it never needs your passphrase.
It works for bids made in **any** hsd-compatible wallet, not just Namehold,
because the derivation is the hsd standard.

For the full guide — including recovering bids placed in other hsd-compatible
wallets — see [RECOVER_LOST_BIDS.md](RECOVER_LOST_BIDS.md).

### Advanced actions

Click **Show all actions** at the bottom of the Name Actions modal to reveal
Update, Renew, Transfer, Finalize, Cancel, and Revoke — only relevant for
names you already own.

---

## 9. DNS records editor

The DNS editor lives inside the Name Actions modal (Register and Update
actions) for owned names.

### Prefilled from the chain

When you open the editor for a name that already has records on-chain, the
wallet fetches them via `getnameresource` and **prefills the editor** so you
can edit rather than re-enter. Prefill needs a synced node; without one, the
editor opens empty.

### Record types

| Type | Fields |
|------|--------|
| **TXT** | One or more text strings |
| **NS** | Nameserver hostname |
| **DS** | Key tag, algorithm, digest type, digest hex |
| **GLUE4** / **GLUE6** | Hostname + IPv4/IPv6 address |
| **SYNTH4** / **SYNTH6** | Hostname + IPv4/IPv6 address (synthesized) |

Click **+ Add record** to append a row. Records are optional — you can register
a name with no records and add them later via an Update.

### Raw-JSON advanced view

A toggle switches the editor into raw-JSON mode for unusual records or when
you already have a records object you want to paste. Example:

```json
[{"type":"TXT","txt":["hello world"]},{"type":"NS","ns":"ns1.example."}]
```

---

## 10. Managing names you own

In the Name Actions modal for an owned name, click **Show all actions**:

| Action | What it does |
|--------|--------------|
| **Update** | Replace the on-chain DNS records (uses the DNS editor). |
| **Renew** | Extend the name's expiry. |
| **Transfer** | Start a transfer to another Handshake address. Enters a `TRANSFER` covenant. |
| **Finalize** | Complete a transfer after the lockup period (mainnet: ~2 days). |
| **Cancel** | Revert a pending transfer before it's finalized. |
| **Revoke** | Permanently burn the name (irreversible). |
| **Buy with payment** | Finalize a transfer AND pay the seller in a single transaction (atomic swap). |

All of these need the signer unlocked and a synced node.

### Batch operations

Select multiple names in the Owned Names table using the checkboxes (or the
header checkbox to select all). A batch action bar appears at the bottom:

- **Renew Selected** — renew all selected names in a single transaction.
- **Reveal Selected** — bulk reveal all selected names' bids (enabled when
  every selected name is in REVEAL phase).
- **Redeem Selected** — bulk sweep losing-bid coins from selected names.
- **Finalize Selected** — bulk finalize outgoing TRANSFERs whose lockup has
  expired.

Each batch action opens a **confirmation modal** showing the count, estimated
fee, and a collapsible list of the selected names. Cancel closes without
broadcasting; Confirm signs + broadcasts the draft in one step.

Batch operations use hsd's `createbatch` RPC, which handles consensus limits
automatically (chunking to stay under block-size limits).

### Paid name swaps

To sell a name for HNS (**Sell with payment** flow):
1. Open the name in **Manage** → **Sell with payment** section.
2. Enter the buyer's address, your price (HNS), and confirm. This transfers
   the name to the buyer with a lockup period recorded as a saved offer.
3. Wait for the buyer to broadcast their finalize-with-payment tx.
4. Once the buyer's tx confirms, the app verifies it (checks the payment
   output matches your offer) and marks the offer paid — HNS lands in your
   wallet atomically.

To buy a name:
1. Wait for the seller to transfer the name to your address (name shows TRANSFER state).
2. Click **Buy with payment** → enter seller's address + amount.
3. Review the draft → sign → broadcast.
4. The name is finalized and the seller is paid in the same transaction.

---

## 10b. Name watchlist

The **Watchlist** page (sidebar) lets you track names you don't own:

- **Add a name** — enter the name in the input field and click "Add".
- **From name modals** — the Manage and Info modals now show an
  **Add to Watchlist** / **Remove from Watchlist** toggle in the header, so
  you can start tracking a name straight from any auction view.
- **Columns** — the table shows:
  - **Name** — clickable (opens the name info modal). An **Owned** badge
    appears next to names owned by the active wallet profile.
  - **State** — current auction phase badge (Opening, Bidding, Reveal, Closed,
    etc.) fetched live from the node or explorer.
  - **Countdown** — time until the next phase transition, e.g. "Bidding opens
    in 42 blocks (~7h)" or "Expires in 2400 blocks". Uses the same helpers as
    the Auctions view.
  - **Highest bid** — the highest revealed bid so far (HNS). Visible during
    Reveal and Closed phases.
  - **Expires** — days until the name expires (colour-graded: green > 90d,
    yellow 30–90d, red ≤ 30d). Matches the Renewals view thresholds.
  - **Tags** — inline-editable comma-separated tags.
  - **Added** — date the name was added to the watchlist.
- **Tags** — click any tag cell to edit a comma-separated list of tags (e.g.
  `auctions, expiring-soon, competitors`). Tags are stored per name and
  round-trip through CSV export/import.
- **Remove** — click the "Remove" button to stop tracking a name.
- **CSV export / import** — export your watchlist to
  `name,tags,notes,added_at,state,expiry` CSV, and re-import into any wallet.
  Import is additive (existing rows are preserved).

The watchlist is stored in the local SQLite database (`watched_names` table).
A daemon-written cache (`watched_name_states`) provides instant column data on
first page open; live RPC refreshes every 30 seconds while the page is visible.

### Watchlist notifications (background alerts)

Enable in **Settings → Watchlist notifications**. When on, the background sync
daemon (`namehold-syncd`) polls each watched name every ~60 seconds and fires
OS notifications when:

- A name **enters BIDDING** (you can now bid).
- A previously CLOSED name **becomes available again** (re-opened for auction).
- A name in OPENING is **about to start bidding** within the configured lead
  time (default 144 blocks ≈ 1 day).
- The **highest bid** on a name crosses a global threshold you set (in HNS).

Alerts fire even when the Namehold app is closed (as long as background sync
is enabled). Each event fires only once per auction episode; re-auctions on the
same name are treated as fresh episodes.

**Settings fields:**

| Field | Default | Notes |
|-------|---------|-------|
| Enable watchlist notifications | Off | Opt-in; triggers OS permission prompt on first enable |
| Bidding-soon lead time (blocks) | 144 | ~1 day before BIDDING opens |
| Highest-bid alert threshold (HNS) | (blank = off) | Alert when any watched name's highest bid crosses this value upward |

**macOS note:** The daemon binary runs unbundled, so notifications may show a
generic sender icon rather than the Namehold app icon. The alert text is
unaffected.

---

## 11. Node control

All node settings live under **Settings → Connections**.

### Fields

| Field | Default | Notes |
|-------|---------|-------|
| **Explorer base URL (reads)** | `https://e.hnsfans.com` | For node-free reads. |
| **Node RPC URL (sending)** | `http://127.0.0.1:12037` | Mainnet. Testnet 13037, regtest 14037. |
| **Node RPC API key** | (empty) | Match hsd's `--api-key`. |
| **Node data directory (`hsd --prefix`)** | (system default) | Use **Browse…** to pick. |
| **hsd binary path** | (auto) | Only needed if hsd isn't on PATH. |
| **Autostart HSD when the app launches** | **on** | Toggle off to keep hsd manual. |
| **Sync in background** | **on** | When enabled, a background daemon syncs your wallet every 60 seconds, even when the app is closed. When disabled, only manual Sync (or the app's auto-sync while running) refreshes your data. |
| **Node mode** | **Full node** | **SPV** (lightweight, faster sync, explorer-dependent) or **Full node** (requires ~15GB, indexes all addresses). SPV mode is read-only — cannot send transactions. |
| **Explorer fallback URL** | (empty) | When primary explorer is unreachable, automatically tries this URL. Leave empty to disable failover. |

### Background sync

The **Sync in background** checkbox (Settings → Connections) keeps your wallet
data fresh without the app being open.

- **When ON (default):** hsd stays running after you close the app. A background
  daemon (`namehold-syncd`) wakes every 60 seconds and syncs all wallet profiles
  (UTXOs, name states, transactions) from hsd into the local database. The next
  time you launch the app, it adopts the running hsd and picks up where the daemon
  left off — so your data is always current.
- **When OFF:** hsd is stopped when you close the app (the classic behavior).
  Only manual **Sync**, or the app's built-in auto-sync while the app is running,
  refreshes your data.
- **Crash recovery:** if the daemon dies, the app detects it on the next startup
  and respawns it (as long as the toggle is ON).
- **Read-only:** the daemon never signs transactions or broadcasts — it only reads
  from hsd and writes sync data to the local database. Your keys stay locked in the
  encrypted vault.

### SPV mode (lightweight)

The **Node mode** dropdown (Settings → Connections) lets you choose between:

- **Full node** (default): hsd runs with `--index-address --index-tx`. Requires
  ~15GB disk space and initial sync time. Supports sending and full local data.
- **SPV** (lightweight): hsd runs with `--spv`. Downloads only block headers
  (~几十MB). Faster initial sync, less disk usage. **Read-only** — cannot send
  transactions. Balance and name data come from the explorer.

When SPV mode is active:
- The **StatusStrip** shows "Explorer (SPV)" to indicate data comes from the explorer.
- **Sending is blocked** with a clear message: "SPV mode cannot send transactions."
- **Explorer failover** is available — set a fallback URL in Settings for when the
  primary explorer is unreachable.

**To enable SPV mode:**
1. Go to **Settings → Connections**
2. Change **Node mode** from "Full node" to "SPV"
3. **Save settings** — hsd restarts with `--spv` flag
4. Data reads now come from the explorer; sending is blocked

**To switch back to full node:**
1. Change **Node mode** back to "Full node"
2. **Save settings** — hsd restarts with `--index-address --index-tx`
3. Full sync begins (may take time if the chain has advanced significantly)

### Start, stop, status

Below the settings, the **NodeControl** panel shows a live status dot
(Connected · Starting · Stopped · Syncing %), the read source (**Local** or
**Explorer**), the data directory and hsd version. Buttons:

- **Start hsd** — spawns hsd with the required flags (`--index-address
  --index-tx`). If hsd is already running, it adopts it via RPC.
- **Stop hsd** — stops the node.
- **Re-sync node data** — appears only when the app detects an
  **index mismatch** (hsd's existing chain was synced without `--index-address`).
  hsd cannot add indexes retroactively, so this moves the old `blocks/`,
  `chain/`, `tree/` aside and resyncs with the right flags. Expect this to
  take hours the first time.

### Ports

| Network | RPC port |
|---------|---------|
| Mainnet | 12037 |
| Testnet | 13037 |
| Regtest | 14037 |

### Troubleshooting background sync

**The background daemon isn't syncing.** Check that hsd is running (the status
dot is Connected). Restart the app — it respawns the daemon on startup when
"Sync in background" is ON. The daemon writes its process ID to
`~/.namehold/syncd.pid` while alive.

**hsd is still running after I closed the app.** This is expected when "Sync in
background" is enabled — the daemon keeps hsd alive so it can sync in the
background. To stop hsd, either disable "Sync in background" (hsd is then stopped
on the next app close) or click **Stop hsd** in Settings → Connections.

**I want to stop background sync.** Uncheck **Settings → Connections → "Sync in
background"**. The daemon exits immediately, and hsd is stopped the next time you
close the app.

---

## 12. Move from Namebase

**Move from Namebase** is a guided helper for migrating off the custodial
Namebase service. It is **not** the wallet's core function — a wallet works
standalone.

1. Paste your Namebase session cookie to connect.
2. Review your custodial domains, spotting **expiring soon** entries.
3. **Transfer** names out to your own wallet address (Namebase-initiated).
4. **Withdraw HNS** to your address.
5. **Compare** your imported inventory against what Namebase still holds.

On-chain finalization of the transfers uses the same node-backed write path
as the rest of the wallet (Signer unlock → sign → broadcast).

---

## 13. System Tray

Starting with v0.4.0, Namehold places an icon in your system tray (menu bar on
macOS, notification area on Windows/Linux). This keeps the app, hsd node, and
background sync daemon running even after you close the main window.

### Close to tray

Enabled by default (**Settings → System Tray → "Close to tray"**). Clicking the
window's close button hides Namehold to the tray instead of quitting. Click the
tray icon or choose **Open Namehold** from the tray menu to restore the window.
Use **Quit** from the tray menu to fully exit.

### Launch at login

**Settings → System Tray → "Launch at login"** registers Namehold to start
automatically when you log in (LaunchAgent on macOS, Run key on Windows,
`.desktop` on Linux). Pairs well with "Close to tray" for an always-available
menu-bar experience.

### Tray menu

Right-click the tray icon for: **Open Namehold**, live **node status** with a
Start/Stop toggle, a **Sync in background** checkbox, and **Quit**. The tray
icon reflects node state (normal / syncing / stopped) and adapts to light/dark
menu bars on macOS.

---

## 14. Data location and macOS quarantine

### Data location

All app data lives in one SQLite file in your home folder (pairs with hsd's
`~/.hsd`), on every platform:

```
~/.namehold/portfolio.db
```

It holds your wallet profiles, the encrypted vault, the local chain cache, and
sync state.

When background sync is running, the daemon also writes its process ID to
`~/.namehold/syncd.pid` (removed when the daemon stops).

### macOS quarantine

The macOS build is not code-signed. On first launch macOS may show:

> "Namehold" can't be opened because Apple cannot check it for malicious software.

Remove the quarantine flag:

```bash
xattr -cr /Applications/Namehold.app
```

Then open the app normally.

---

## 14. Security

- **Non-custodial.** Your keys live on your device, encrypted at rest with
  Argon2id + AES-256-GCM. Nothing is custodied.
- **Secrets never reach the web layer.** Passphrase entry and recovery-phrase
  display happen in a small Rust-owned secure window, and signing happens in
  Rust. Your JavaScript UI never sees them.
- **Local-first.** No cloud, no telemetry. Keys and secrets stay on your
  device. By default, balance/name lookups go to the public HNSFans explorer,
  which therefore sees your wallet addresses and the names you track; running
  your own hsd node keeps those lookups fully local. The only other outbound
  HTTP is to your configured node.
- **Localhost-first.** Namehold connects to hsd on `127.0.0.1` by default;
  non-localhost URLs are allowed but warned about.
- **Auto-lock.** The unlocked signer times out after a configurable idle
  period (Settings → Advanced → Signer session timeout, default 15 min).
- **Background sync is read-only.** When "Sync in background" is enabled, the
  `namehold-syncd` daemon reads from hsd and writes sync data to the local
  database — it never signs transactions, never broadcasts, and never has access
  to key material. Your keys stay locked in the encrypted vault.
- **What Namehold never does.** Never asks for or transmits your seed phrase,
  never logs passphrases or private keys, never talks to a remote wallet
  service.

---

## 15. Troubleshooting

### Header shows "READ-ONLY" and Send is disabled

Hover the button (or open the Send dialog) to see the exact reason. Common
ones:

- **"Signer locked"** — click **Unlock** on the account bar.
- **"Node not synced (N%)"** — wait for hsd to finish syncing.
- **"hsd not address-indexed"** — hsd was started without `--index-address`.
  Restart hsd from Settings → Connections; the app adds the flag. If your
  chain was already synced without it, use **Re-sync node data**.
- **"Node unreachable"** — check that hsd is running (Settings shows a red dot)
  and the RPC URL/API key match.

### The DNS editor opens empty for a name I know has records

Prefill needs a synced node. If you're on Explorer reads only, the editor
starts empty. Start / wait for hsd and re-open the modal.

### "Cannot retroactively enable indexing" from hsd

hsd cannot add an index to an already-synced chain. In Settings the app shows
the **Re-sync node data** button — this moves the old `blocks/`, `chain/`,
`tree/` aside and resyncs with `--index-address --index-tx`. Expect a few
hours on mainnet.

### Balance shows 0 after import

A freshly imported wallet won't show any HNS until:
1. The node is synced,
2. The address index has caught up,
3. **Sync** has been clicked (or the auto-sync loop has run once).

### Send fails with "Not sent"

The dialog stays open with the exact error. Common causes: not enough HNS to
cover amount + fee, the node lost peers mid-broadcast, or the network
temporarily rejected the tx. Fix the issue and click Sign & Broadcast again —
the draft is still there.

### CSV import shows errors

Check that your CSV has a `Name` column, names don't have leading/trailing
spaces, and duplicate rows are OK (they're updated, not errors).

---

## 16. Auto-update

Starting with **v0.2.0**, Namehold checks for its own updates automatically.
You don't need to visit a website or run a package manager; new signed
builds are delivered and installed in place.

### How it works

- ~30 seconds after launch, the app silently queries GitHub Releases for a
  newer version. If nothing new is available, no UI appears.
- If an update is available, a **banner** appears at the top of the window:
  "Namehold v{version} is available", with **Install now** and **Later** buttons.
- You can also check on demand: **Settings > Updates > Check for updates**.
  The card there also shows your current running version.
- Clicking **Install now** downloads the update with a progress indicator
  ("Downloading... N%"), then reports "Update installed. Restart to finish."
- Click **Restart now** to finish the install. On Windows the app exits
  automatically during install (OS limitation); on macOS and Linux you
  trigger the relaunch yourself.

### Dismissing an update

Click **Later** on the banner to hide it for that specific version. The
update remains available under **Settings > Updates** if you change your
mind, and the banner will reappear when the next version ships.

### Security

Every update bundle is **Ed25519-signed** at release time. Namehold verifies
the signature against a public key embedded in the app binary before
installing anything; unsigned or tampered bundles are rejected. The private
signing key never touches your machine.

### Checking your current version

Open **Settings > Updates**. Your running version is shown at the top of
the card (e.g. "Current version: v0.5.0"). This is currently the only
in-app surface that displays the app version.

---

*Namehold — non-custodial Handshake wallet. See `CHANGELOG.md` for what's new.*
