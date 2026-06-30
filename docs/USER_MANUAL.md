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
8. [Transferring TLDs](#8-transferring-tlds)
9. [Tracking Migration](#9-tracking-migration)
10. [Syncing with Your Wallet](#10-syncing-with-your-wallet)
11. [Renewals](#11-renewals)
12. [DNS Records](#12-dns-records)
13. [Exporting Data](#13-exporting-data)
14. [Security](#14-security)
15. [Troubleshooting](#15-troubleshooting)

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

## 8. Transferring TLDs

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

## 9. Tracking Migration

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

## 10. Syncing with Your Wallet

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

## 11. Renewals

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

## 12. DNS Records

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

## 13. Exporting Data

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

## 14. Security

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

## 15. Troubleshooting

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
| Transfer TLD | Portfolio → Inventory | Write mode + passphrase |
| Sync names | Migration → Sync & Verify | hsd connection |
| View DNS records | Portfolio → DNS | hsd connection |
| View renewals | Portfolio → Renewals | hsd connection |
| Node status | Node | None |

---

*Namehold v0.1.0 — your HNS network wallet*
