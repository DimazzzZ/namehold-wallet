# QA Workflows

This document describes how to test the Namehold wallet across its three execution modes: **browser (web QA)**, **Tauri (native shell)**, and **regtest (deterministic chain)**.

---

## 1. Browser + Playwright (web QA)

### When to use
Background, non-interrupting, repeatable UI regression testing. The browser app runs in a normal Chromium tab and does not steal focus from the editor.

### How it works
- `pnpm dev` starts the Vite dev server.
- The app detects it is **not** inside Tauri and routes all `invoke()` calls through `src/lib/webqa-mock.ts`.
- Playwright drives the full UI against `http://localhost:5173`.

### Commands
```bash
# Terminal 1 — start the dev server
pnpm dev

# Terminal 2 — run Playwright (once tests exist)
npx playwright test
```

### What can be validated
- Navigation and layout rendering
- Form validation (send HNS, bid amounts, transfer addresses)
- Modal flows (NameActionsModal, wallet management, settings)
- Button enable/disable logic (gating by signer lock, node sync state)
- Status badge rendering
- Settings UI (node controls, explorer URL, RPC URL)
- Auction lookup and phase badge display
- Wallet switching and add-wallet flows
- Background sync toggle (**Settings → Connections → "Sync in background"**) —
  checkbox enable/disable, toast feedback, form state revert on failure

### Current limitations
The web mock (`src/lib/webqa-mock.ts`) is **static** — it returns canned data and does not persist state across commands:
- `start_hsd()` returns success but `node_status()` still returns the hardcoded "stopped" state.
- Building/signing/broadcasting a draft does not change the drafts list or transaction history.
- Auction phase transitions are not simulated.
- `set_background_sync_enabled()` returns success but does not actually spawn or
  stop a daemon — the mock reports `is_daemon_alive: false` always, so
  daemon-lifecycle end-to-end flows can only be validated in the Tauri or regtest
  paths below.

**To make browser QA realistic for lifecycle flows**, the mock must be upgraded to a stateful, scenario-driven backend. Until then, browser QA validates **UI structure and interaction patterns**, not end-to-end lifecycle correctness.

---

## 2. Tauri + cua-driver (native shell)

### When to use
Testing native macOS integration — secure window, folder picker, real Tauri IPC, window management.

### Commands
```bash
# Start the Tauri dev build
pnpm tauri dev
```

Then use the `cua-driver` skill for background automation (snapshot → click/type → re-snapshot loop).

### Caveats
- The Tauri window may open on a different macOS Space. The user must switch to that Space before cua-driver can capture the full AX tree.
- The `cua-driver` skill enforces a no-foreground contract — it must not raise or activate any app.

### What can be validated
- Secure wallet creation/import
- Backup phrase reveal
- Folder picker (node data directory)
- Native dialogs
- Real node connection and IPC

### Background sync daemon (end-to-end)

To verify background sync works correctly:

1. **Enable background sync:** Toggle **Settings → Connections → "Sync in
   background"** ON. Confirm the daemon spawns (check `~/.namehold/syncd.pid`
   exists and contains a valid PID).
2. **Close the app:** Verify hsd is still running (e.g., `ps aux | grep hsd` or
   check the PID file).
3. **Mine or wait:** Let the daemon sync a few cycles (every 60s). Optionally
   mine a new block on regtest to give the daemon fresh data to sync.
4. **Reopen the app:** Verify the app adopts the running hsd (no duplicate
   spawn). Check that wallet data is fresh (balances, transaction history).
5. **Disable background sync:** Toggle OFF. Close the app. Verify hsd is now
   stopped (PID file is gone, `ps aux | grep hsd` shows no process).

---

## 3. Regtest (deterministic chain)

### When to use
Full end-to-end lifecycle validation with real hsd RPC. This is the only mode where auction lifecycle transitions actually happen on-chain.

### Prerequisites
1. hsd installed and available on PATH
2. A regtest node running:
   ```bash
   hsd --network=regtest --daemon
   ```
3. The app configured to connect to `http://127.0.0.1:14037` (regtest RPC port).
4. The regtest wallet funded by mining blocks:
   ```bash
   hsd-cli rpc generate 200
   ```

### Auction lifecycle on regtest
```bash
# Mine blocks to advance through auction phases
hsd-cli rpc generate 1    # advance 1 block
hsd-cli rpc generate 720  # advance through OPENING phase
hsd-cli rpc generate 720  # advance through BIDDING phase
hsd-cli rpc generate 1440 # advance through REVEAL phase
```

### What can be validated
- Send HNS between wallets
- Open auction → bid → reveal → register (full lifecycle)
- Transfer name ownership
- Finalize / cancel transfers
- Renew names
- Revoke names
- DNS record updates

---

## Scenario Checklists

### A. Shell & navigation
- [ ] App loads without console errors
- [ ] All sidebar routes render: Wallet, Auctions, Move from Namebase, Settings, Portfolio (if advanced mode)
- [ ] Status strip shows correct wallet/network/signer/node/source badges
- [ ] Beta warning banner visible

### B. Settings & node status
- [ ] Explorer URL field editable and saveable
- [ ] Node RPC URL field editable and saveable
- [ ] Node data directory field + Browse button (Tauri only)
- [ ] Start/Stop hsd buttons reflect node state
- [ ] Sync progress bar shows during sync, 100% when synced
- [ ] Read source label: "Explorer" when node not synced, "Local" when synced
- [ ] Last error shown when node start fails
- [ ] Re-sync button appears on index mismatch
- [ ] **Node mode dropdown** visible when chain_source ≠ explorer
- [ ] **Node mode dropdown** hidden when chain_source = explorer
- [ ] **Explorer fallback URL** field editable and saveable

### B2. SPV mode
- [ ] Switch to SPV mode in Settings → Node mode dropdown
- [ ] hsd restarts with `--spv` flag (check logs)
- [ ] StatusStrip shows "Explorer (SPV)" label
- [ ] Balance loads from explorer (not local cache)
- [ ] Name info loads from explorer
- [ ] Send HNS blocked with "SPV mode cannot send transactions" message
- [ ] Name actions blocked with clear SPV message
- [ ] Switch back to Full node → hsd restarts with `--index-address`
- [ ] Full sync begins after switching back
- [ ] Explorer failover works (set invalid primary, valid fallback)

### C. Wallet flows
- [ ] Receive address displayed with copy button
- [ ] Balance shows confirmed/unconfirmed/spendable
- [ ] Send HNS: address validation (mainnet `hs1…`, regtest `rs1…`, testnet `ts1…`)
- [ ] Send HNS: amount validation, fee display, review step
- [ ] Send HNS: build → sign → broadcast draft lifecycle
- [ ] Signer lock/unlock toggle
- [ ] Wallet switching via dropdown
- [ ] Add wallet: create / import / watch-only paths
- [ ] Manage wallets dialog: switch, reveal phrase, delete

### D. Auction flows
- [ ] Auctions page: name lookup field
- [ ] Lookup any name → NameActionsModal opens
- [ ] Available name: Open Auction button shown
- [ ] Opening phase: waiting for bidding to start
- [ ] Bidding phase: Bid form (bid amount + lockup), countdown timer
- [ ] Reveal phase: Reveal button, unrevealed bids warning
- [ ] Closed/won: Register button
- [ ] Gating: actions disabled when signer locked or node not synced
- [ ] Wallet page "Get a TLD" card links to Auctions

### E. Post-auction name management
- [ ] Update DNS records
- [ ] Renew name
- [ ] Transfer name (enter recipient address)
- [ ] Finalize transfer
- [ ] Cancel transfer
- [ ] Revoke name

### F. Transfers & Namebase migration
- [ ] Transfers view: incoming/outgoing transfers
- [ ] Namebase connection flow
- [ ] Domain import from Namebase
- [ ] Renewals view

---

## Handshake Units Reference

- 1 HNS = 1,000,000 doos (dollarydoos)
- Backend covenant builders expect **integer doos**
- UI should let users enter **HNS values** and convert internally
- Bid = visible bid amount; Lockup = additional hidden amount (total commitment = bid + lockup)

## Auction Lifecycle Reference

```
AVAILABLE → OPENING (5 days / ~720 blocks)
         → BIDDING (5 days / ~720 blocks)
         → REVEAL (10 days / ~1440 blocks)
         → CLOSED (winner can REGISTER)
         → REGISTERED (owner can UPDATE / RENEW / TRANSFER / REVOKE)
```

On regtest, these periods are much shorter and blocks can be mined instantly.

---

```

---

## 4. Rust Backend Coverage Policy

### Goal

Target **≥95% line coverage** for all meaningful backend logic in `src-tauri/`.

### What counts as meaningful code (must be covered)

- Validation and guard logic (network checks, profile resolution, amount checks)
- Database reads and writes (queries, migrations, upserts)
- Orchestration logic (build → sign → broadcast lifecycle, sync flows)
- RPC error mapping and fallback paths (explorer ↔ local node)
- Covenant / name action planning and persistence
- Fee resolution and coin selection
- Session / signer lifecycle rules

### What may be excluded from coverage

Only **thin framework / OS / Tauri glue** that exists purely to wire runtime APIs:

- `src-tauri/src/lib.rs` `run()` entrypoint — `tauri::Builder`, plugin registration, `generate_handler!`
- `WebviewWindowBuilder` open/close wiring in `secure_prompt.rs`
- Direct `std::process::Command::new(...).spawn()` boundaries in `node.rs`
- Any code that is fundamentally a framework adapter with no decision logic

### Exclusion mechanism

Use `#[coverage(off)]` via `cargo-llvm-cov`'s nightly cfg support. In `src-tauri/Cargo.toml`:

```toml
[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ['cfg(coverage,coverage_nightly)'] }
```

In the crate root or module:

```rust
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

#[cfg_attr(coverage_nightly, coverage(off))]
fn thin_framework_glue() { /* OS / Tauri adapter only */ }
```

Every exclusion **must** have a short comment explaining why it is excluded.

### Rules

1. **Never blanket-exclude entire command modules** (`names.rs`, `tx.rs`, `read.rs`, `secure_wallet.rs`). If a module mixes logic and glue, **refactor first** (behavior-preserving), then test the extracted logic.
2. **Prefer refactoring over exclusion.** Extract testable helpers from mixed modules; keep `#[tauri::command]` wrappers thin.
3. **Document every exclusion.** Each `coverage(off)` annotation needs a comment: what it is and why it cannot be unit-tested.
4. **Re-audit exclusions periodically.** If a framework API becomes testable (e.g., Tauri adds mock support for window events), remove the exclusion and add a test.

### Testing hierarchy

| Layer | What to test | Tooling |
|-------|-------------|---------|
| Unit | Pure helpers: validation, derivation, fee math, error mapping | `#[test]` / `#[tokio::test]` |
| Integration | Command orchestration: DB + mockito RPC + Tauri mock app | `tauri::test::mock_builder`, `mockito`, in-memory SQLite |
| Native | Secure window, folder picker, real Tauri IPC | `pnpm tauri dev` + `cua-driver` |
| E2E chain | Send, auction lifecycle, covenant acceptance | `hsd --network=regtest` |

### Regenerating coverage

The saved report at `src-tauri/coverage/html/index.html` may be stale. Always regenerate before making decisions:

```bash
cd src-tauri && cargo llvm-cov --open
```

---

## Key Files

| Area | File |
|------|------|
| Web mock backend | `src/lib/webqa-mock.ts` |
| App entry / routing | `src/App.tsx` |
| Wallet page | `src/components/WalletView.tsx` |
| Auctions page | `src/components/AuctionsView.tsx` |
| Name actions modal | `src/components/NameActionsModal.tsx` |
| Settings page | `src/components/Settings.tsx` |
| Auction phase logic | `src/lib/auction.ts` |
| Read queries | `src/queries/read.ts` |
| Wallet queries | `src/queries/wallet.ts` |
| Node queries | `src/queries/node.ts` |
| Frontend types | `src/types/index.ts` |
| Rust read commands | `src-tauri/src/commands/read.rs` |
| Rust name commands | `src-tauri/src/commands/names.rs` |
| HNSFans adapter | `src-tauri/src/providers/hnsfans.rs` |
| Backend types | `src-tauri/src/hsd/types.rs` |
| Existing tests | `src/components/__tests__/` |
| Regtest docs | `docs/REGTEST_TESTING.md` |
