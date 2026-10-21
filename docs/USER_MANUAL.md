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
    - [Buying names on the Market](#buying-names-on-the-market)
    - [Selling names through Shakedex](#selling-names-through-shakedex)
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

The sidebar has seven top-level sections:

| Section | Purpose |
|---------|---------|
| **Wallet** | Balance, receive, send, recent transactions, owned names. Default page. |
| **Activity** | Transaction history and pending activity. |
| **Auctions** | Look up a name, place bids, reveal, register, see active auctions. |
| **Market** (marked New) | Browse LearnHNS Market listings, import a listing file, and buy a listed name. |
| **Watchlist** | Names you track without owning them. |
| **Move from Namebase** | Retired: the legacy Namebase shut down on 1 October 2026. Explains the shutdown and imports history you exported earlier. |
| **Settings** | Connections, node control, backups, notifications, advanced options. |

The digit keys `1`–`7` open the sections in this order.

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
reveals the account xpub. It is only useful when another service needs to compute your Handshake addresses; otherwise leave it closed.

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
3. **Node synced** — the blocks hsd has applied have caught up to the best
   header it knows about; `verification_progress` only corroborates that (see
   "When the wallet calls a node synced" in [NODE_SETUP.md](NODE_SETUP.md)).
4. **Address-indexed** — hsd was started with `--index-address` (required to
   discover your unspent coins). The wallet asks hsd which indexes it runs,
   which hsd reports from version 7.0, so the node must be hsd 7.0 or newer.

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

### Several bids on one name

You can bid on the same name as many times as you like while it is in
Bidding, each bid with its own value and lockup. Every bid goes to a fresh
address and gets its own commitment, so each is revealed and reclaimed on its
own. The Name Actions modal's header reads "Latest bid … · lockup … · N of
yours", and the bids panel lists every bid on the name with yours tinted.
**Reveal** and **Redeem** act on all of your bids on that name at once, and
the confirm dialog sums every output they carry.

A bid you did not reveal in time loses its whole lockup: Handshake lets a bid be spent only by a reveal, and only a reveal can be redeemed, so the coin stays on-chain where nothing can move it. If you bid on a name again in a later auction, the guided panel names that stranded lockup and its amount, once your synced node confirms the coin is still unspent, so the money is not mistaken for part of the new bid. If nobody reveals at all, the auction lapses and the name reads Available to Open again; the guided panel names your unrevealed bid's lockup as stranded there too, once your node confirms the coin is still unspent. Without a synced node to ask, the wallet does not guess: the panel says that some of your bids on the name could not be checked, and to sync with your node to see whether a lockup was lost. Redeem a losing reveal before the name expires. A name the winner revoked can still be redeemed from, and Redeem stays on offer while it reads Revoked; a revoked name expires a fixed number of blocks after the revoke (about 29 days on mainnet), any other name at the end of its renewal window. Redeem stops at expiry, not when a new auction opens: in the last block before the name expires the wallet no longer offers it, nor shows the name as a lost bid to redeem, since a redeem would be mined after the expiry, and from then on Handshake no longer redeems a reveal of that auction, which the guided panel names as stranded too.

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
| **Renew** | Extend the name's expiry. Not right after the name was registered, renewed or finalized: the network accepts a renewal only 36 blocks (about 6 hours) later on mainnet and testnet, 5 on regtest, and until then the button says how many blocks are left. |
| **Transfer** | Start a transfer to another Handshake address. Enters a `TRANSFER` covenant. |
| **Finalize** | Complete a transfer after the lockup period (mainnet: ~2 days). |
| **Cancel** | Revert a pending transfer before it's finalized. |
| **Revoke** | Permanently burn the name (irreversible). |

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
- **Transfer Selected** — transfer all selected names to one recipient
  address, entered in the action bar.

Each batch action opens a **confirmation modal** showing the count, the amount
the transaction moves (every output except change — a batch reveal or redeem
carries one per bid), the estimated fee, and a collapsible list of the
selected names. Cancel closes without
broadcasting; Confirm signs + broadcasts the draft in one step.

A batch is built in the wallet as one transaction with one txid, so it either
all lands or none of it does. There is no chunking and no `createbatch` RPC in
hsd. The wallet caps a batch at 100 names, well under hsd's per-transaction
covenant limits (300 OPENs, 600 UPDATEs, 600 RENEWs), and per-block limits are
the same as per-transaction ones — so a full batch is never refused for
carrying too many covenants.

### Buying names on the Market

The Market is the fourth sidebar section (hotkey `4`, right after Auctions, marked New). It lists the names that sellers have put up for sale through Shakedex, taken from the LearnHNS Market, and lets you buy one with the wallet you already have.

A listing is either a fixed price (Buy Now) or a reverse auction whose price steps fall over time to a floor. The Browse tab shows them in one list: listings you imported first, then those of the LearnHNS Market. Each row shows the name, the price now (the price the wallet buys at), the next price for a reverse auction ("↓ 1,100 HNS in ~6 h", or "At its floor"; "Fixed price" otherwise) and whether your node checked it. Those times are approximate, because they are measured against your node's median time: a step becomes valid once its lock time, rounded down to a multiple of 512 seconds, is below that median time, so it can be listed as "now" up to about eight and a half minutes before its own lock time. Above the list, "Find a name" searches by name, the All / Fixed price / Price drops buttons filter by kind, and the sort puts the lowest price first (the default), the soonest price drop first, or the newest first (imports, then the market's own order). A row can carry one tag: "Bought" for a name you are buying, "Expires soon" when the name expires about a month after the earliest finalize, or "From file" / "From link" for an import.

Click a row to see it in the panel beside the list (below it in a narrow window); the first verified row is selected for you. The panel shows the price, and for a reverse auction when it drops next, its floor and its price steps from now on; for a fixed price, the seller's listing expiry if it has one. That date is for information only: the signed price stays valid on chain until the name is sold or the listing cancelled, as the panel says. Under "Checked by your node" it lists what was verified: the seller's signature over the locked name, that the lock coin is unspent, and until when the name is registered, or a warning when it expires soon after you can finalize.

Every LearnHNS Market listing is verified against your own node before it can be bought. A listing your node could not answer for (it did not respond, or took too long) stays in the list with no price and "Couldn't check": Retry, in the row or as "Check again" in the panel, asks your node again. Listings that cannot be bought at all are not listed; a line at the foot of the list counts them by reason ("5 unavailable listings hidden: 3 sold, 1 failed the check, 1 not valid yet") and expands to each name with its reason. The market is loaded a hundred listings at a time: Load more adds the next hundred to the same list, until it reads "All LearnHNS listings loaded". While the first listings are being checked, the list shows placeholder rows. With an SPV or Explorer connection, or while your local node is still syncing or not answering, the list says listings can't be checked and why, instead of showing them; listings you imported stay in the list. A listing you just bought stays in the list, tagged Bought, with "Purchase sent" in its panel.

The LearnHNS Market lists mainnet names only. On testnet and regtest the list says there is no LearnHNS Market on this network, and only importing a listing file or pasted JSON works; a market.learnhns.com link is refused there.

You can also buy a listing that is not on the market. Import listing…, at the top right of the page, opens a dialog where you paste the listing JSON or a market.learnhns.com/listing/<name> link, or choose a listing file. The listing is verified the same way; once your node verifies it, the dialog closes and the listing is added to the list, selected, with the search and filter cleared. A listing that cannot be bought is not added: the dialog says why (already sold or cancelled, failed the check, not valid yet, expires before it could be finalized, or your node could not check it). A market.learnhns.com link is the market's own listing, so its market fee is handled exactly as for a row of the market; a file or pasted JSON never pays a market fee. A listing this device locked itself, in any of its profiles, is not bought: its panel says "This is your own listing: cancel it instead (My listings), buying it would only cost you network fees." and Buy stays disabled.

Buying needs a local node, or a remote node with "Allow sending via remote node" turned on in Settings, and a wallet created or restored from a recovery phrase: with a Ledger, a watch-only wallet or one imported from an extended private key, the panel says "Shakedex works with a recovery-phrase wallet for now." and Buy stays disabled. On mainnet, buying also needs "Allow buying and selling names on mainnet through Shakedex (experimental)" ticked in Settings. Until it is, a banner at the top of the Market says buying on mainnet is off, and both its "Turn on buying" button and the panel's button in place of Buy tick that setting; nothing is bought until you then press Buy. Testnet and regtest are always allowed, and Finalize is never gated.

The panel prices the purchase before you buy: the price to the seller, the LearnHNS fee with its share of the price, the network fee and the total, to six decimals. The address the market fee is paid to is shown in the secure confirmation window, not in the panel. A listing whose fee address is unusable on this network still verifies; its fee line says it names no valid fee address, and no market fee is paid. The fee is ticked for you only when it is the fee LearnHNS itself publishes; for any other fee it starts unticked with a warning, and you decide whether to pay it. Your choice is kept for that listing while the app is open. "Fee rate" opens a field for this purchase's fee rate in doos/kvB (empty uses your default; a whole number of at least 1,000). When Buy cannot be pressed, the line above it says why: for example that you have less available than the total, by how much, and for a reverse auction when the price drops next. Buy names the name and the total ("Buy .coffee for 255 HNS"). It opens the secure window, where you unlock the signer as for any other spend. Just before the purchase is sent, the wallet checks the price step against your node's median time again. If a cheaper step has become valid meanwhile, nothing is sent: the panel says the price changed, the prepared transaction is discarded, and the panel shows the new figures for you to buy again. The same happens if your node does not report a median time at that moment. Cancelling the secure window also discards the prepared purchase, and the listing can be bought again straight away. Once the purchase is sent, the panel reads "Purchase sent", the row is tagged "Bought", and the purchase appears under Your purchases.

Your purchases, a card at the top of Browse, lists every purchase bought and not yet finalized: the price paid, how far its transfer lockup is (288 blocks, about two days, on mainnet and testnet; 10 on regtest), how many blocks are left (with roughly how long that is on mainnet and testnet; regtest mines on demand, so no time is given), and Finalize once it is over. Finalize there works whatever the mainnet setting says, and a purchase leaves the card once its finalize is sent.

On the Market, `/` focuses the search, `i` opens Import, `j` and `k` (or the arrow keys) move through the list, and `Enter` presses the panel's button.

A purchase appears in Owned Names with its own status: "Unconfirmed purchase" until the transaction confirms, then "Awaiting finalize · N blocks" while the transfer delay runs (288 blocks, about two days, on mainnet and testnet; 10 on regtest; the Market panel states it for your network), then "Ready to finalize" with a Finalize button, and "Finalize · waiting for a block" once the finalize is sent. The name becomes yours only after you finalize it, and Finalize is never gated by the mainnet setting. When it is done, update the DNS records, because the name keeps the seller's. Finalize before the name expires: the Market panel warns when the name expires soon after the finalize becomes possible. A purchase whose name expires unfinalized is lost: the price was paid, but the name is not yours. The row leaves Owned Names, and Activity marks the purchase "Lost" and shows the reason under it; so does any purchase that ends without the name, paid or not.

### Selling names through Shakedex

Sell is in a name's actions, in the Ownership section next to Transfer. It opens the Lock dialog, the first of a sale's two steps: choose Buy Now (one price) or Reverse auction (the price falls step by step to a floor), and, on mainnet, whether to publish the listing on LearnHNS Market. The price, or a reverse auction's schedule, is not asked here: you set it at Finalize & sign, once the transfer lockup is over (288 blocks, about two days, on mainnet and testnet; 10 on regtest). Lock name builds a transfer of the name to its lock address; the name stays at your own address until that transfer is finalized into the lock. Locking needs the unlocked signer: on a locked wallet Sell is disabled until you unlock it, and if the signer locks while the dialog is open, the dialog asks you to unlock it again and carries on. Sign and send the lock confirms it in the secure window, as for any other spend. Back, or closing the dialog, after the lock is built discards the transaction that was not sent.

The dialog states what locking costs: "Once the name is finalized into the lock, its DNS records cannot be changed and it cannot be renewed; the listing lasts at most one renewal window from that finalize, which renews the name."

"Publish on LearnHNS Market" is ticked by default and is offered on mainnet only. Its note says: "Publishing makes the name, its price and its lock public on LearnHNS Market, and the market sees your IP address every time the wallet checks or uploads the listing." Unticked, the listing is shared as its file only. On testnet and regtest there is no box: the dialog says "Shared as a file only: LearnHNS Market lists mainnet names only".

A name that would expire before its transfer into the lock could be finalized (within the transfer lockup plus a day) is refused when you choose Lock name, with the reason and the advice to renew it first, and nothing is built. A name that expires within about six months can be locked, and the dialog then shows, before anything is signed, how many blocks are left and that Finalize & sign must be done before then; the finalize into the lock renews the name, so that expiry does not cut the listing short.

Selling follows the buying gates. It needs a local node, or a remote node with sending allowed, and a wallet created or restored from a recovery phrase: with a Ledger, a watch-only wallet or one imported from an extended private key, Sell and Lock name are disabled with "Shakedex works with a recovery-phrase wallet for now". On mainnet, selling also needs "Allow buying and selling names on mainnet through Shakedex (experimental)" ticked in Settings; until it is, Sell and Lock name are disabled with "Selling names through Shakedex on mainnet is experimental: enable it in Settings". Testnet and regtest are always allowed. Sell is also disabled, with the reason, whenever the name cannot be transferred, for example while it is already locked for sale.

#### My listings

Market → My listings (the tab next to Browse) lists every listing of the active wallet, one row each: the name, Buy now or Reverse auction (a lock restored by name shows no mode and reads "Locked — listing details unknown"), its state (Locking, Ready to Finalize & sign, Finalizing into the lock, Listed for sale, Sale pending, Sold, Cancelling, Cancelled · finalize to bring it home, Coming home, Cancelled, Lock aborted, Expired in the lock) and where it stands on LearnHNS Market. A Listed row also shows its prices at your node's median time: "Price X HNS" for Buy Now, "Now X HNS · next Y HNS in ~T · floor Z HNS" for a reverse auction with every price step behind "All N price steps", and the listing's "Listed until" date. No other state shows a price. Without a full or remote node the row says the current price was not checked; a node that did not answer, or stored prices that cannot be read, are named with the reason.

A name in the lock cannot be renewed, so a listing lives at most until the name's registration runs out, and buyers stop seeing it earlier: once the name would expire before a purchase could be finalized (the transfer lockup plus a day), the Market hides it. When that is less than 30 days away for a Listed row or a restored lock, the row says so in one amber line: "Buyers stop seeing this listing in ~N days, when the name gets too close to expiring to finalize a purchase. Cancel it and bring the name home to renew it, then list it again." (on regtest and simnet, where blocks are mined on demand, "in N blocks"). Past that point it reads "Buyers no longer see this listing: the name is too close to expiring to finalize a purchase. Cancel it and bring the name home to renew it." The count is read from your node; without a full or remote node the row says nothing about it.

The market status reads, for a published listing: "On LearnHNS Market as a pending listing", "On LearnHNS Market", "Replaced on the market by someone else's copy — re-uploaded", "Not on market — retrying" with the time of the next try and the market's answer, "LearnHNS Market refused the listing" with its reason (sent again only when the price or expiry changes; refused while still pending, it is uploaded once the finalize into the lock is mined), "Not on market: a signed price does not verify over the lock coin your node reports, so nothing was uploaded", and, once the listing has ended, "LearnHNS Market was told this listing ended". An unpublished listing reads "Not published on LearnHNS Market — shared as a file only" on mainnet, and "Shared as a file only: LearnHNS Market lists mainnet names only" elsewhere.

A published listing goes on LearnHNS Market as a pending listing once its lock is sent, and with its price once the finalize into the lock is mined. The wallet then keeps it there: about hourly it compares the market's copy with the listing's current price and uploads it again when the copy differs, for example after someone else uploaded their own copy, after Lower price, or when a reverse auction's price steps down, since LearnHNS Market shows one price at a time. Once a sale or a cancel is mined, the market is told at the next sync. This runs at every sync of the app and of the background sync daemon (`namehold-syncd`), which publishes listings already signed but never signs or sends a transaction.

When a purchase of the listing reaches your node's mempool, the row reads Sale pending, and Sold once the purchase is mined; Sold names stay in the list. While the purchase is only in the mempool, Cancel listing and Lower price are refused before anything is asked, because the node would not take them.

Each state shows only its own actions: Finalize & sign while Locking or Ready to Finalize & sign; Lower price, Cancel listing, Export listing file and (unpublished, on mainnet) Publish on LearnHNS Market while Listed; Export listing file while a sale is pending or a cancel is under way; Lower price while Cancelling, disabled with "a cancel of this listing is not mined yet: its price cannot be lowered"; Finalize home once a cancel is mined; Cancel listing and Import its listing file for a restored lock. An action that cannot run now is shown disabled with the reason, never hidden. Signing actions need a recovery-phrase wallet and a node that can send, as for buying, and ask you to unlock the signer when it is locked; Export listing file works with any wallet. A listing whose transaction was not sent (for example a lock whose broadcast the wallet refused before sending) offers Send (Sign and send when it was not signed yet) and Discard on its row; Discard puts the listing back where it was and says where, and a Locking listing goes with its lock. A transaction the node did not take, or did not answer for, may still be held by the node: its row offers Send again and no Discard. After a failed send, the error says when the transaction stays in My listings; a lock refused at send time because the name would expire first is deleted with its listing, and the error says why.

Lower price signs one more, lower price after the secure window shows it. A price that is not below the current one is refused before anything is asked: "a price can only be lowered; raising it needs a cancel". The dialog says that the earlier, dearer signatures stay valid until the lock coin is spent, so anyone holding the listing file can still use them.

Cancel listing says, before anything is signed, that anyone holding the listing file can still buy the name at its current price until the cancel is mined (when a cheaper price step is still ahead, as in a falling reverse auction: at any price step valid by then, the cheaper one too); that a purchase already in your node's mempool keeps the node from taking the cancel, although it still answers with a transaction id; that once the cancel is mined and the transfer lockup is over you finalize it to bring the name home; and, for a published listing, when LearnHNS Market is told. Sign and send the cancel signs it in the secure window, which shows the current price and the fee, and sends it. Once it is mined and the lockup is over, Finalize home builds, signs and sends the finalize that brings the name back to your wallet.

Export listing file saves the listing's stored file where you choose, to share with a buyer. Publish on LearnHNS Market turns publishing on later for a Listed listing that was not published, on mainnet only and with the mainnet selling setting on, and never off; it goes on the market at the next sync. Its note is the Lock dialog's: publishing makes the name, its price and its lock public, and the market sees your IP address.

#### Finalize & sign

Once the transfer lockup is over the row reads Ready to Finalize & sign, and Finalize & sign opens the form where the price is chosen, the last chance to change it before anything is signed. A Buy Now asks for its one price in HNS. A reverse auction asks for its schedule: a start price, a floor price below it, the number of prices (2 to 1000) and the minutes between them (at least 15); the form then sums it up ("6 prices from 10 to 5 HNS, one every 60 min; the last becomes valid ~5 h after the first"). Until every field of the schedule is filled the form says "a reverse auction needs its schedule: a start price, a floor, the number of steps and the time between them". A price or a schedule the wallet would refuse is refused in the form, in the same words, before anything is asked: a price of 0, below the dust limit of 0.001 HNS, above the money supply or with more than 6 decimals; fewer than 2 or more than 1000 prices, prices less than 15 minutes apart, a floor not below the start, or prices that would not strictly fall. No price becomes valid before its time: every price after the first is rounded up to the chain's 512-second unit, so it can become valid up to about eight and a half minutes after its nominal time. A schedule whose last price comes more than a year after the first is allowed, with the warning "The last price comes more than a year after the first, past the listing file's expiry date." The form says that each signature lets anyone buy the name at its price until the listing is cancelled and the cancel is mined, and that a price can be lowered later, never raised. Finalize & sign then shows the secure window, which lists the finalize fee and every price, with when it becomes valid, before anything is signed; the form itself lists no prices. Confirmed, the wallet signs the finalize into the lock and every price at once and sends the finalize; the listing becomes Listed for sale once it is mined. If the send fails, the error says when the signed finalize stays in My listings to be sent from its row. Finalize & sign needs the same wallet and node as the other signing actions, but not the mainnet selling setting, since the listing exists already.

#### Find my locks and Restore a lock by name

A wallet restored from its recovery phrase looks for the names it had locked for sale once, by itself: at the first sync that ends with the wallet unlocked, it reads its own address history from your node and restores every name still in this wallet's lock, as "Locked — listing details unknown" in My listings, with one message saying how many it found ("Found 2 names locked for sale · restored in My listings"). It needs a recovery-phrase wallet, a synced node that can send, with the address index (`--index-tx` and `--index-address`); with the wallet locked, My listings says that names possibly listed from this phrase were found and offers Unlock. A lock still on its way in, a name sold, cancelled or expired, and a lock cancelled or bought are skipped without a message. A node without the index, or one still syncing, runs no search, and it is tried again at the next sync; so is a search that could not read every address or name, or that found history at the newest addresses the wallet watches, before the next sync has gone past them. After one search has finished, it runs again only when you press "Find my locks" in My listings ("No locked names found" when there is nothing new, or "Some addresses or names could not be read from your node. Try again later." when that search could not read everything), so a lock made later from another device of the same phrase is not taken behind your back. Without the index, or for a name the search skipped, use the manual way. Under the list, "Restore a lock by name" takes the name of a listing it had (when the wallet reads the typed name differently, the line under the field says which name it restores, "Restores .<name>"): the wallet checks on your node that the name sits in this wallet's lock, finalized into it, and adopts it as "Locked — listing details unknown"; anything else (an expired or revoked name, a name not in this wallet's lock, a name still on its way into the lock, a lock already cancelled or bought) is refused with the reason. It needs a recovery-phrase wallet, a node that can send and the unlocked signer. A restored lock can be cancelled at once. Import its listing file, on its row, takes the listing file saved for that lock, pasted or chosen on disk, and makes the lock a full listing again, Listed for sale with its prices; a file for another lock key or another lock coin, paying an address that is not this wallet's, or naming a market fee is refused with the reason. A file paying one of this wallet's first 1000 receive addresses is accepted even when that address lies beyond the ones the restore provisioned, and the address is then added to the wallet, so the payment of a sale is seen.

#### On Owned Names and in Activity

A name you are selling shows its listing's state on its Owned Names row, in place of what the name needs next: "Locking · N blocks" while the transfer lockup runs (counted once the lock transfer is mined; "Locking" before that), then the states of My listings ("Ready to Finalize & sign", "Listed for sale", "Cancelled · finalize to bring it home", and so on), and "Sold" for a week after a sale. A lock restored by name reads "Locked — listing details unknown". A name in your lock that is no longer a coin at your own addresses still has its row, at the lock address. The row's My listings button opens Market › My listings, where the listing's actions are; while the listing is open, the name's own Update, Transfer, Renew and Revoke are disabled with "this name is locked for sale through Shakedex". Once a cancelled listing's name is home, the row is an ordinary name again.

With deadline notifications on (Settings › Notifications), the app sends a "Ready to finalize" notification when a listing's transfer lockup is over ("<name>: the transfer lockup is over — Finalize & sign to lock it in and set its price", repeated daily while it waits) and when a cancel's lockup is over ("<name>: the cancel's transfer lockup is over — finalize it to bring the name home").

Activity names the selling transactions while they are drafts: "Lock for sale" (the transfer into the lock), "Finalize into lock" (the finalize Finalize & sign sends), "Cancel listing" (the cancel) and "Finalize cancel" (the finalize that brings the name home). Each can be picked in Activity's action filter.

### Paid name swaps

Not available. The wallet once offered "Sell with payment" and "Buy with
payment"; both were withdrawn on 2026-09-21 because the shape they implemented
could not do what the names promised. A transfer's coin stays at the seller's
address, so only the seller can finalize — which left "Buy with payment"
pressable only by the party with nobody to pay — and nothing about the
transaction was atomic, so "one transaction" meant one wallet funding both
halves of its own trade.

Buying a name that someone has listed through Shakedex is a different mechanism and is available; see "Buying names on the Market" above. So is selling through Shakedex; see "Selling names through Shakedex" above. Selling a name any other way still means transferring it and being paid
separately, with the trust that implies. Offers recorded before the buttons
were withdrawn are listed at the foot of Activity under "Paid swap offers",
read-only: the name, the buyer's address, the price and whether it was
claimed.

`docs/specs/2026-09-21-paid-name-swaps.md` has the consensus rules behind this
and what a working implementation would need.

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
| **Chain source** | **Local full node** | Which node the wallet sends through: **Local full node** (hsd on this device, full indexes), **SPV — lightweight, read-only** (headers only, explorer for data), **Remote node** (someone else's hsd RPC), or **Read-only (never send)**. Reads are not routed by this selector: they come from the node whenever it is synced and on your wallet's network, and from the explorer otherwise. |
| **Node RPC URL** | `http://127.0.0.1:12037` | When chain source is Local full node or Remote node. Mainnet 12037, testnet 13037, regtest 14037. |
| **Node RPC API key** | (empty) | For remote nodes: match the remote hsd's `--api-key`. Ignored for local nodes. |
| **Allow sending via remote node** | **off** | When chain source is Remote node, enable this to broadcast signed transactions to the remote node. Off by default for safety. |
| **Explorer base URL** | (network-dependent) | Mainnet: `https://e.hnsfans.com`. Testnet/regtest have no default explorer — set one explicitly if you have one. |
| **Explorer fallback URL** | (empty) | When the primary explorer is unreachable, automatically tries this URL. Leave empty to disable failover. |
| **Node data directory (`hsd --prefix`)** | (system default) | Use **Browse…** to pick. Local hsd only. |
| **hsd binary path** | (auto) | Only needed if hsd isn't on PATH. Local hsd only. |
| **Autostart HSD when the app launches** | **on** | Toggle off to keep hsd manual. Local hsd only. |
| **Sync in background** | **on** | When enabled, a background daemon syncs your wallet every 60 seconds, even when the app is closed. Local full node only. |

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
- **Never signs or broadcasts:** The background daemon never signs or broadcasts; it publishes listings already signed to LearnHNS Market. It reads from hsd, writes sync data to the local database and, for a listing you chose to publish on mainnet, keeps it on LearnHNS Market. Your keys stay locked in the encrypted vault.

### SPV mode (lightweight)

The **Chain source** selector (Settings → Connections) offers SPV beside the
full node:

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
2. Change **Chain source** from "Local full node" to "SPV — lightweight, read-only"
3. **Save settings** — hsd restarts with `--spv` flag
4. Data reads now come from the explorer; sending is blocked

**To switch back to full node:**
1. Change **Chain source** back to "Local full node"
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

The legacy custodial Namebase platform (sunset.namebase.io) closed on 1 October 2026. Every request to it now returns "Namebase has shut down", so this screen can no longer connect with a session cookie, list your custodial domains, transfer names out or withdraw HNS.

What the screen does now:

1. Explains the shutdown and links to Namebase's sunset guide. According to Namebase, names and HNS still held there can only be recovered through their manual recovery process, which involves extra verification, longer processing and fees. Contact Namebase for it; the wallet cannot do it for you.
2. Imports an account-history CSV you exported from Namebase before the shutdown (**Upload CSV**), so your past activity stays visible. "Fetch from Namebase" is disabled because there is nothing left to fetch.

If you connected before the shutdown, any remaining call reports the shutdown rather than an expired session; **Logout** clears the stored cookie.

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
  your own hsd node keeps those lookups fully local. The only other outbound HTTP is to your configured node and, when you use the Market, to LearnHNS Market (browsing and buying, and publishing a listing you chose to publish on mainnet).
- **Localhost-first.** Namehold connects to hsd on `127.0.0.1` by default;
  non-localhost URLs are allowed but warned about.
- **Auto-lock.** The unlocked signer times out after a configurable idle
  period (Settings → Advanced → Signer session timeout, default 15 min).
- **Background sync never signs or sends.** The background daemon never signs or broadcasts; it publishes listings already signed to LearnHNS Market. When "Sync in background" is enabled, the `namehold-syncd` daemon reads from hsd and writes sync data to the local database; it never has access to key material. Your keys stay locked in the encrypted vault.
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
- **"Your node doesn't report its indexes (hsd older than 7.0)"** — the node is
  an hsd older than 7.0, which cannot say whether it has the address index.
  Update hsd to 7.0 or newer.
- **"Your node's status couldn't be read"** — the node answered the sync check but not its status (`GET /`), so the wallet cannot tell whether it has the address index. Check that hsd is running and reachable at the RPC URL, then Refresh.
- **"Node unreachable"** — check that hsd is running (Settings shows a red dot)
  and the RPC URL/API key match.

### The balance shows 0 with "Your node can't report this wallet's coins"

The last sync could not read this wallet's coins from your node, so the balance is what the wallet last stored, which is 0 for a wallet never synced through an indexed node. The line under the balance gives the reason the sync recorded: most often hsd was started without `--index-address` (restart it from Settings → Connections), or it is older than 7.0 and cannot say whether it has the index (update it). The next sync through a node that can report the coins removes the line.

### Activity says "Your node does not have --index-tx enabled"

Activity reads the history with hsd's address and transaction indexes, and the banner names the one your node lacks (`--index-address`, `--index-tx`, or both). Restart hsd with `--index-address --index-tx`; nodes the app starts already have both.

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

The dialog stays open with the exact error. Common causes: not enough HNS to cover amount + fee, or the node could not be reached mid-broadcast. After sending, the wallet looks the transaction up on the node: if the node does not have it, the dialog says "The node did not take the transaction". hsd does not say why; most often its coins were already spent by another transaction. The coins stay held, and Activity shows the send as waiting until the node is checked again. Fix the issue and click Sign & Broadcast again — the draft is still there. A send already sent, or one marked dropped or failed (its coins were released), cannot be signed or sent again: build a new one. A send marked dropped or failed that is mined after all, within 72 hours, turns confirmed by itself.

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
