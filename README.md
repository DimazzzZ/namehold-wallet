# Namehold - your HNS network wallet

Local desktop app for managing Handshake TLD migration from Namebase. Built with Tauri v2, React, TypeScript, and SQLite.

## Features

- **TLD Inventory**: Import, tag, filter, sort, and track all your TLDs
- **Migration Tracking**: Track transfer status for each TLD through the migration workflow
- **Wallet Integration**: Connect to local hsd node to verify ownership and check balances
- **Batch Management**: Group TLDs into migration batches for organized transfers
- **Sync & Verification**: Compare wallet-owned names against your imported inventory
- **CSV Import/Export**: Flexible import with column mapping, export filtered views
- **Audit Log**: Every operation is logged for full traceability

## Security

- **Read-only by default**: Write actions are disabled until explicitly enabled
- **No seed phrases**: The app never asks for or stores private keys or seed phrases
- **Local only**: All data stays on your machine. No cloud, no telemetry
- **Localhost wallet**: Default wallet API is 127.0.0.1 only. Non-localhost URLs trigger warnings
- **Confirmation dialogs**: All write actions require explicit confirmation on mainnet

## Prerequisites

- [Node.js](https://nodejs.org/) v18+
- [pnpm](https://pnpm.io/) v8+
- [Rust](https://www.rust-lang.org/tools/install) 1.77+
- [hsd](https://github.com/handshake-org/hsd) (optional, for wallet integration)

## Quick Start

```bash
# Clone and install
git clone <repo-url>
cd namehold-wallet
pnpm install

# Run in development mode
pnpm tauri dev
```

## Setting Up hsd (Handshake Node)

### Install hsd

```bash
npm install -g hs-client
```

### Run hsd with wallet

```bash
# Mainnet
hsd --api-key=<your-api-key>

# Testnet
hsd --testnet --api-key=<your-api-key>

# Regtest (for testing)
hsd --regtest --api-key=<your-api-key>
```

### Default Ports

| Network | Node API | Wallet API |
|---------|----------|------------|
| Mainnet | 12037    | 12039      |
| Testnet | 13037    | 13039      |
| Regtest | 14037    | 14039      |

### Configure in App

1. Open Settings in the app
2. Set Wallet API URL (e.g., `http://127.0.0.1:12039`)
3. Set Node API URL (e.g., `http://127.0.0.1:12037`)
4. Enter your API key
5. Set Wallet ID (default: `primary`)
6. Select network (mainnet/testnet/regtest)

## CSV Format

Import TLDs from a CSV file. Supported columns:

```csv
Name,Staked,Category,Tags,Notes
crypto,true,Premium,"high_value,operational",High-value TLD
wallet,false,Finance,"medium_value",Finance TLD
test,false,Test,"low_value,test",Migration test
```

- **Name**: TLD name (required). Leading dots are stripped automatically.
- **Staked**: `true`/`1`/`yes`/`staked` = staked, anything else = unstaked
- **Category**: Free text category
- **Tags**: Comma-separated tags (stored as JSON array)
- **Notes**: Free text notes

Staked TLDs are automatically set to `do_not_touch_staked` status.

## Migration Statuses

| Status | Description |
|--------|-------------|
| `not_started` | No action taken yet |
| `namebase_transfer_requested` | Transfer initiated in Namebase |
| `waiting_transfer_tx` | Waiting for transfer transaction |
| `transfer_seen_on_chain` | Transfer TX detected on blockchain |
| `waiting_finalize` | Waiting for finalization |
| `finalized_owned` | TLD is owned by your wallet |
| `failed_or_stuck` | Transfer failed or stuck |
| `do_not_touch_staked` | Staked TLD - do not migrate |

## Recommended Workflow

1. **Import all TLDs** from CSV
2. **Tag 5 TLDs as staked**, verify they show as `do_not_touch_staked`
3. **Create test batch** with 1 low-value TLD
4. **Initiate transfer** in Namebase for that TLD
5. **Update status** in the app to track progress
6. **Sync with wallet** to verify arrival
7. **Graduate to small batches** (5-10 TLDs) after successful test
8. **Proceed with larger batches** once confident
9. **Do high-value TLDs last**, after all test batches succeed
10. **Withdraw HNS** from Namebase only after all unstaked TLDs are safely received

## Build for Production

```bash
pnpm tauri build
```

Output: `src-tauri/target/release/bundle/`
- macOS: `.app` and `.dmg`
- Windows: `.msi`
- Linux: `.AppImage`

## Database Location

- macOS: `~/.namehold/portfolio.db`
- Windows: `~/.namehold/portfolio.db`
- Linux: `~/.namehold/portfolio.db`

## Tech Stack

- **Tauri v2** - Desktop shell
- **React 19 + TypeScript** - Frontend
- **Vite** - Build tool
- **TanStack Table + Virtual** - High-performance table
- **TanStack Query** - Async state management
- **Zustand** - Client state
- **Zod** - Validation
- **SQLite (rusqlite)** - Local database
- **reqwest** - HTTP client for hsd API
- **Tailwind CSS** - Styling
