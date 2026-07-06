# Namehold — User Manual

Your local desktop wallet for managing Handshake TLDs and HNS.

---

## Navigation

Namehold uses a consolidated sidebar with six primary sections:

| Section | Purpose | Sub-tabs |
|---------|---------|----------|
| **Overview** | Portfolio and infrastructure summary at a glance | — |
| **Portfolio** | Manage your TLDs | Inventory · Batches · Renewals · DNS |
| **Migration** | Track Namebase transfers and verify ownership | Namebase · Sync & Verify |
| **Wallet** | HNS balance, send, receive | — |
| **Auctions** | Acquire new Handshake TLDs | — |
| **Node** | hsd node connection and status | — |
| **Settings** | Connection, write mode, preferences | — |

The header badges show the active network (e.g. `mainnet`) and whether the app
is in `READ-ONLY` or `WRITE` mode.

## Table of Contents

1. [Getting Started](#1-getting-started)
2. [Connecting to hsd](#2-connecting-to-hsd)
3. [Importing TLDs](#3-importing-tlds)
4. [Viewing Your Portfolio](#4-viewing-your-portfolio)
5. [Receiving HNS](#5-receiving-hns)
6. [Sending HNS](#6-sending-hns)
7. [Receiving TLDs](#7-receiving-tlds)
8. [Acquiring New TLDs (Auctions)](#8-acquiring-new-tlds-auctions)
9. [Transferring TLDs](#9-transferring-tlds)
10. [Tracking Migration](#10-tracking-migration)
11. [Syncing with Your Wallet](#11-syncing-with-your-wallet)
12. [Renewals](#12-renewals)
13. [DNS Records](#13-dns-records)
14. [Exporting Data](#14-exporting-data)
15. [Security](#15-security)
16. [Troubleshooting](#16-troubleshooting)

---

## 1. Getting Started

### What is Namehold?

Namehold is a local desktop app that helps you manage Handshake TLDs (top-level domains) and HNS coins. It connects to your local hsd node to verify ownership, check balances, and perform transactions.

### Prerequisites

Before using Namehold, you need:

- **hsd** — the Handshake full node software, running on your computer
- **A wallet** — created inside hsd (the `primary` wallet by default)

### Install hsd

```bash
npm install -g hs-client
```

### Start hsd

```bash
# Mainnet
hsd --api-key=YOUR_SECRET_API_KEY

# Testnet (for testing)
hsd --testnet --api-key=YOUR_SECRET_API_KEY

# Regtest (for development)
hsd --regtest --api-key=YOUR_SECRET_API_KEY
```

Replace `YOUR_SECRET_API_KEY` with a strong random string. You can generate one with:
```bash
node -e "console.log(require('crypto').randomBytes(32).toString('hex'))"
```

### Launch Namehold

Open the Namehold app. On first launch you'll be guided through creating or
importing a wallet. Once a wallet is selected, you'll land on the **Overview**
page with empty data.

---

## 2. Connecting to hsd

Namehold needs to connect to your local hsd node to read wallet data and perform transactions.

### Step-by-step

1. Click **Node** or **Settings** in the left sidebar
2. Fill in the connection details:

| Field | Default | Description |
|-------|---------|-------------|
| **Wallet API URL** | `http://127.0.0.1:12039` | The wallet REST API address |
| **Node API URL** | `http://127.0.0.1:12037` | The node REST API address |
| **API Key** | (empty) | The `--api-key` you set when starting hsd |
| **Wallet ID** | `primary` | The wallet name inside hsd |
| **Network** | `mainnet` | mainnet, testnet, or regtest |

3. Click the **Wallet** page in the sidebar to verify the connection shows "Connected"

### Default ports

| Network | Wallet Port | Node Port |
|---------|-------------|-----------|
| mainnet | 12039 | 12037 |
| testnet | 13039 | 13037 |
| regtest | 14039 | 14037 |

### Security warning

If you enter a non-localhost URL (anything other than `127.0.0.1` or `localhost`), the app will show a warning. Only use local connections for security.

---

## 3. Importing TLDs

### CSV format

Create a CSV file with your TLDs. Example:

```csv
Name,Staked,Category,Notes
crypto,true,Premium,High-value TLD
wallet,false,Finance,Finance TLD
defi,false,Finance,DeFi related
nft,false,Art,NFT marketplace
test,false,Test,Migration test
```

**Supported columns:**
- **Name** (required) — the TLD name, with or without leading dot
- **Staked** — `true`, `1`, `yes`, or `staked` = staked; anything else = unstaked
- **Category** — free text (e.g., Premium, Finance, Art)
- **Tags** — comma-separated tags (e.g., `high_value,test`)
- **Notes** — free text notes

### How to import

1. Go to **Portfolio → Inventory**
2. Click **Import CSV**
3. Select your CSV file
4. The app will import all rows and show a summary

### What happens on import

- Staked TLDs are automatically set to **Do Not Touch** status
- Unstaked TLDs start as **Not Started**
- Duplicate TLD names are updated (not duplicated)
- An audit log entry is created

---

## 4. Viewing Your Portfolio

### Overview

The **Overview** page shows:
- Key portfolio metrics (total TLDs, in wallet, pending migration, expiring soon)
- A status breakdown of TLDs by migration status
- Recent activity from the audit log
- System status (Node, Wallet, Balance)

### Portfolio → Inventory

The main table (under **Portfolio → Inventory**) shows all your TLDs with:
- Name, Status, Category, HNS State, Expiration, Notes, Updated date

**Filters:**
- **Staked/Unstaked** dropdown
- **Status** dropdown (e.g., Not Started, Finalized, etc.)
- **Sort by** Name, Status, Category, or Updated
- **Search** box (searches name, notes, category)

**Bulk actions** (select rows with checkboxes):
- Update Status — change migration status for multiple TLDs
- Set Tags — assign tags to multiple TLDs
- Create Batch — create a migration batch from selected TLDs
- Transfer — send a TLD to another address (write mode only)

---

## 5. Receiving HNS

To receive HNS, you need to share your wallet's receive address.

1. Go to the **Wallet** page
2. Find the **Receive Address** section
3. Click **Copy** to copy the address to your clipboard
4. Share this address with the sender

The address starts with `rs1q...` (mainnet) or `ts1q...` (testnet).

### Refreshing your balance

Your balance updates automatically every 30 seconds. To refresh manually, navigate away from the Wallet page and back, or restart the app.

---

## 6. Sending HNS

Sending HNS requires **Write Mode** to be enabled.

### Step-by-step

1. Go to **Settings** and enable **Write Mode**
2. Enter your **Wallet Passphrase** in Settings (stored in memory only, lost on restart)
3. Go to the **Wallet** page
4. Click **Send HNS**
5. Enter:
   - **Destination Address** — the recipient's Handshake address
   - **Amount** — in HNS (e.g., `1.5`)
   - **Wallet Passphrase** — if not saved in Settings
6. Review the warning message
7. Click **Send HNS**

### Important notes

- The passphrase is your hsd wallet passphrase (set when you created the wallet)
- Transactions cannot be undone
- The app converts HNS to dollarydoos automatically (1 HNS = 1,000,000 dollarydoos)
- An audit log entry is created for every send

---

## 7. Receiving TLDs

TLDs arrive in your wallet when someone transfers them to you (e.g., from Namebase).

### How to check if TLDs arrived

1. Go to **Migration → Sync & Verify**
2. Click **Sync Now** (or **Compare Names** to preview without updating)
3. The app fetches all names from your wallet and compares them with your imported inventory
4. Matched names are automatically marked as **Finalized**

### What the sync shows

- **Matched** — TLDs in both your inventory and wallet
- **Extra in Wallet** — names in your wallet but not in your inventory
- **Not in Wallet** — TLDs in your inventory but not yet received

---

## 8. Acquiring New TLDs (Auctions)

You can register new Handshake TLDs directly from Namehold through the
on-chain Vickrey auction process. The entire flow is guided step-by-step
inside the app.

### How Handshake auctions work

Every unclaimed Handshake name goes through a four-phase auction:

| Phase | Duration | What happens |
|-------|----------|--------------|
| **Opening** | ~1 day (720 blocks) | The name enters the auction. No bids are accepted yet. |
| **Bidding** | ~5 days (1 440 blocks) | Anyone can place sealed bids. You choose how much to bid and how much to lock up (the lockup can exceed the bid to obscure the real value). |
| **Reveal** | ~2 days (720 blocks) | Bids are revealed. If you bid and don't reveal, you lose your locked funds. |
| **Closed** | — | The winner is resolved. The highest bidder can register the name. |

### Step-by-step: getting a new TLD

1. Click **Auctions** in the sidebar.
2. Type the name you want (without the leading dot) and click **Look up**
   (or press Enter).
3. The **Name Actions** modal opens and fetches the current auction state.
4. Follow the guided action shown at the top of the modal:

   | Current state | Guided action | What you provide |
   |---------------|---------------|------------------|
   | Available / Opening | **Open Auction** | Nothing extra — just confirm. |
   | Bidding | **Place Bid** | Bid amount (HNS) and lockup amount (HNS). |
   | Reveal | **Reveal Bid** | Nothing extra — just confirm. |
   | Closed (you won) | **Register** | Optional DNS records for the TLD. |

5. Each step builds a transaction draft, asks you to unlock the signer
   (if locked), signs the draft, and broadcasts it.
6. After broadcast, the modal shows the transaction ID and the phase
   updates automatically on the next refresh.

### Bid and lockup amounts

- **Bid** — the actual value you are willing to pay. If you are the
  highest bidder, this amount is deducted from your wallet.
- **Lockup** — the total amount locked during the bidding phase. It can
  be equal to or higher than your bid. A higher lockup makes it harder
  for others to guess your real bid. Any lockup exceeding the bid is
  returned to you after the reveal phase.

Both values are entered in **HNS** (the app converts to dollarydoos
internally).

### DNS records

After winning an auction, you can configure DNS records for your new TLD
during the Register step. The modal provides a simple row editor:

- Click **+ Add record** to add a new record.
- Each row has a **Type** selector (TXT, A, AAAA, CNAME, NS, MX, SRV)
  and a **Value** field.
- Records are optional — you can register without any and add them later
  via the Portfolio → DNS page.

### Advanced actions

Click **Show all actions** at the bottom of the Name Actions modal to
reveal additional actions such as Update, Renew, Transfer, Finalize,
Cancel, and Revoke. These are only relevant for names you already own.

### Locked-in-auctions balance

While you have active bids, the Wallet page shows a **Locked in Auctions**
card displaying the total HNS currently locked in bids. This amount is
not spendable until the auction ends and funds are released.

### Reveal warning

If you have names in the **Reveal** phase, a yellow alert banner appears
at the top of the Wallet page reminding you to reveal your bids before
the reveal window closes.

---

## 9. Transferring TLDs

To send a TLD to another address (e.g., to a buyer):

### Step-by-step

1. Enable **Write Mode** in Settings
2. Enter your **Wallet Passphrase** in Settings
3. Go to **Portfolio → Inventory**
4. Select the TLD you want to transfer (check the box)
5. Click **Transfer** in the bulk action bar
6. Enter the **Destination Address**
7. Enter your **Wallet Passphrase** (if not saved)
8. Review the warning
9. Click **Transfer**

### Important notes

- Transfers are on-chain transactions and cannot be undone
- Only one TLD can be transferred at a time
- The transfer creates a TRANSFER covenant on the blockchain
- The recipient must finalize the transfer to complete it

---

## 10. Tracking Migration

Migration tracking helps you organize the process of moving TLDs from Namebase to your own wallet.

<p align="center">
  <img src="assets/namebase-migration.png" alt="Namebase migration" width="700" />
</p>

### Migration statuses

| Status | Meaning |
|--------|---------|
| **Not Started** | No action taken yet |
| **Transfer Requested** | Transfer initiated in Namebase |
| **Waiting TX** | Waiting for the transfer transaction |
| **TX Seen** | Transfer transaction detected on-chain |
| **Waiting Finalize** | Waiting for finalization |
| **Finalized** | TLD is owned by your wallet |
| **Failed/Stuck** | Transfer failed or stuck |
| **Do Not Touch** | Staked TLD — do not migrate |

### Updating statuses

1. Select TLDs in the inventory
2. Click **Update Status**
3. Choose the new status

### Creating batches

Batches help you organize TLDs into migration groups (e.g., "Test Batch 1", "High Value").

1. Select TLDs in the inventory
2. Click **Create Batch**
3. Enter a batch name
4. The batch appears under **Portfolio → Batches**

### Recommended workflow

1. **Start with 1 low-value test TLD** — verify the process works
2. **Then 5-10 TLDs** — small batch
3. **Then larger batches** — once confident
4. **Do high-value TLDs last** — after all test batches succeed
5. **Keep HNS on Namebase** until all unstaked TLDs are received
6. **Withdraw HNS last** — after all TLDs are safely in your wallet

---

## 11. Syncing with Your Wallet

The **Migration → Sync & Verify** tab compares your imported inventory against what your wallet actually owns.

### Sync Now

Click **Sync Now** to:
1. Fetch all names from your wallet
2. Match them against your imported TLDs
3. Update matched TLDs to **Finalized** status
4. Store a wallet snapshot (balance, address, name count)

### Compare Names

Click **Compare Names** to see the diff without updating any statuses:
- **Matched** — in both inventory and wallet
- **Missing** — in inventory but not in wallet (expected for non-finalized TLDs)
- **Extra** — in wallet but not in inventory

### Wallet Snapshots

Each sync stores a snapshot of your wallet state. You can view the history at the bottom of the **Sync & Verify** tab.

---

## 12. Renewals

The **Portfolio → Renewals** tab shows TLDs with known expiration data.

### What it shows

- TLD name, status, name state
- Days until expire (color-coded: red <30d, yellow <90d, green >90d)
- Expiration block height
- Last synced time

### How expiration data is populated

Expiration data comes from hsd during sync. Run a sync to populate this data.

### Renewal tracking

Renewal tracking is **read-only** in the current version. You cannot renew TLDs directly from the app yet.

---

## 13. DNS Records

The **Portfolio → DNS** tab shows resource records for names owned by your wallet.

### How to view records

1. Select an owned name from the dropdown
2. Click **Fetch Records**
3. The app shows:
   - Name state, height, days until expire
   - Resource records (NS, DS, TXT, GLUE4, GLUE6, SYNTH4, SYNTH6)

### Record types

| Type | Description |
|------|-------------|
| **NS** | Nameserver delegation |
| **DS** | DNSSEC delegation signer |
| **TXT** | Text records |
| **GLUE4** | IPv4 glue records |
| **GLUE6** | IPv6 glue records |
| **SYNTH4** | Synthetic IPv4 records |
| **SYNTH6** | Synthetic IPv6 records |

---

## 14. Exporting Data

### Export from TLD Inventory

1. Click **Export CSV** at the top of the inventory
2. Choose where to save the file
3. The export includes all visible columns

### Export from Renewals

1. Click **Export CSV** on the **Portfolio → Renewals** tab
2. Exports TLDs with expiration data

### What's exported

- Name, Status, Staked, Category, Tags, Notes
- HNS Received, Transfer TX, Finalize TX
- Name State, Expires At Height, Last Synced
- Created, Updated timestamps

---

## 15. Security

### Read-only mode (default)

By default, Namehold is in **read-only mode**. This means:
- You can view all data
- You can import/export CSV
- You can create batches
- You **cannot** send HNS, transfer TLDs, or perform any write operations

### Write mode

To enable write operations:
1. Go to **Settings**
2. Toggle **Write Mode** to Enabled
3. A warning will appear

### Wallet passphrase

Your wallet passphrase is needed for all write operations. It is:
- Stored in **memory only** (not saved to disk)
- **Lost on app restart** (you'll need to re-enter it)
- Never logged or exposed in the UI

### Localhost only

Namehold connects to hsd on `127.0.0.1` by default. If you configure a non-localhost URL, the app will show a security warning.

### What Namehold never does

- Never asks for or stores your seed phrase
- Never stores private keys
- Never logs API keys or passphrases
- Never connects to remote servers (unless you configure it)

---

## 16. Troubleshooting

### "Disconnected" on Wallet page

- Make sure hsd is running
- Check that the API key in Settings matches your hsd `--api-key`
- Verify the wallet URL and port are correct
- Make sure you're using the right network (mainnet/testnet/regtest)

### "Write mode is disabled"

- Go to Settings and enable Write Mode
- Write Mode must be enabled for send, transfer, renew, and finalize operations

### "Enter wallet passphrase"

- Enter your hsd wallet passphrase in Settings
- Or enter it directly in the send/transfer dialog
- The passphrase is the one you set when creating your hsd wallet

### CSV import shows errors

- Check that your CSV has a "Name" column
- Make sure TLD names don't have leading/trailing spaces
- Check for duplicate rows (duplicates are updated, not errors)

### Sync shows "Extra in Wallet"

- These are names in your wallet that aren't in your imported inventory
- You can import them by adding them to your CSV and re-importing

### Balance shows 0

- Make sure hsd is fully synced with the blockchain
- Check that you're looking at the right wallet ID
- Verify the network matches (mainnet/testnet/regtest)

### Transaction fails

- Check that your wallet has enough HNS for the transaction + fee
- Make sure the wallet passphrase is correct
- Verify the destination address is valid
- Check that hsd is connected to the network

---

## Quick Reference

| Action | Page | Requirements |
|--------|------|-------------|
| View summary | Overview | None |
| View TLDs | Portfolio → Inventory | None |
| Import CSV | Portfolio → Inventory | None |
| Export CSV | Portfolio → Inventory / Renewals | None |
| Create batch | Portfolio → Inventory / Batches | None |
| Check balance | Wallet | hsd connection |
| Copy receive address | Wallet | hsd connection |
| Send HNS | Wallet | Write mode + passphrase |
| Acquire new TLD | Auctions | Write mode + passphrase |
| Transfer TLD | Portfolio → Inventory | Write mode + passphrase |
| Sync names | Migration → Sync & Verify | hsd connection |
| View DNS records | Portfolio → DNS | hsd connection |
| View renewals | Portfolio → Renewals | hsd connection |
| Node status | Node | None |

---

*Namehold v0.1.0 — your HNS network wallet*
