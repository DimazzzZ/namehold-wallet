# Namehold Wallet E2E Tests

End-to-end tests for the Tauri desktop app, covering runtime-only code paths that cannot be unit-tested.

## What's tested here

- **lib.rs** — Tauri app lifecycle, window creation, menu/tray initialization
- **secure_prompt.rs** — Secure webview window, IPC dispatch, prompt orchestration
- **secure_wallet.rs** — Wallet creation/import flow, session management
- **daemon/mod.rs** — Background sync daemon lifecycle (if applicable)
- **commands/updates.rs** — Auto-update flow (platform-specific)

## Running tests locally

### Prerequisites

- Rust + Cargo (stable + nightly for coverage)
- Node.js 22+ and pnpm 11.17+
- WebDriver (platform-specific):
  - **Linux**: `tauri-driver` (installed via npm)
  - **macOS**: Safari WebDriver (built-in)
  - **Windows**: Edge WebDriver (built-in)

### Build the app

```bash
cd /path/to/namehold-wallet
pnpm install
pnpm tauri build  # or `cargo tauri build` from src-tauri/
```

### Run E2E tests

```bash
cd e2e
pnpm install
pnpm test
```

The test runner will:
1. Start `tauri-driver` (Linux) or the platform WebDriver (macOS/Windows)
2. Launch the built Tauri app from `../src-tauri/target/release/namehold-wallet`
3. Run the specs in `specs/`
4. Report results and exit

### Debug a specific test

```bash
cd e2e
pnpm test:debug
```

## Test structure

- `specs/lib.e2e.ts` — App lifecycle and window management
- `specs/secure-prompt.e2e.ts` — Secure webview window
- `specs/secure-wallet.e2e.ts` — Wallet creation/import flow

## CI integration

E2E tests run on GitHub Actions (see `.github/workflows/e2e.yml`):

1. Build the Tauri app (`cargo tauri build --release`)
2. Install E2E dependencies (`pnpm install` in `e2e/`)
3. Run tests (`pnpm test`)
4. Upload screenshots/logs on failure

## Extending tests

To add a new E2E test:

1. Create a new spec file in `specs/` (e.g., `specs/node-lifecycle.e2e.ts`)
2. Use WebdriverIO's API to interact with the app:
   ```typescript
   const button = await browser.$("button[data-testid='create-wallet']");
   await button.click();
   const result = await browser.$("div[data-testid='wallet-created']");
   await result.waitForDisplayed({ timeout: 5000 });
   expect(await result.isDisplayed()).toBe(true);
   ```
3. Run `pnpm test` to verify

## Known limitations

- **Ledger HID tests**: Require a real Ledger device or a mock HID layer (not yet implemented)
- **Secure prompt tests**: Currently smoke tests; full interactive tests require a test harness for the secure window
- **Platform-specific**: macOS/Windows tests must run on those platforms (CI runs on Linux)
- **App startup time**: Tests use 5–60 second timeouts; adjust `mochaOpts.timeout` in `wdio.conf.ts` if running on slow hardware

## References

- [Tauri WebDriver Example](https://github.com/tauri-apps/webdriver-example)
- [WebdriverIO Docs](https://webdriver.io/)
- [Tauri v2 Testing Guide](https://v2.tauri.app/develop/testing/)
