# Namehold Wallet E2E Tests

End-to-end smoke tests for the Tauri desktop app: they launch the real binary under WebDriver and check that it starts, renders and survives window events.

## What's tested here

- `specs/lib.e2e.ts` (`src-tauri/src/lib.rs`) — the main window opens with a title containing "Namehold", the React root renders, the window is at least 1280×800 and can be resized to 1024×600 and back.
- `specs/secure-prompt.e2e.ts` (`src-tauri/src/commands/secure_prompt.rs`) — smoke only: the main window still answers with its title. No secure prompt is opened.
- `specs/secure-wallet.e2e.ts` (`src-tauri/src/commands/secure_wallet.rs`) — smoke only: the React root is displayed and the title answers. No wallet is created or imported.

The background daemon (`src-tauri/src/daemon/`) and the update flow (`src-tauri/src/commands/updates.rs`) are not exercised here.

## Running tests locally

### Prerequisites

- Linux. The suite is configured for Linux only (`platformName: "linux"` in `wdio.conf.ts`) and drives the app through `tauri-driver`, which wraps WebKitGTK's `WebKitWebDriver`.
- Rust + Cargo, Node.js 22+ and pnpm 11.17+
- `tauri-driver`, a Cargo binary: `cargo install tauri-driver --version 2.1.0 --locked`
- `WebKitWebDriver`: on Ubuntu 26.04 the package is `webkitgtk-webdriver` (older releases call it `webkit2gtk-driver`), plus the Tauri build dependencies listed in `.github/workflows/e2e.yml`
- A display: a desktop session, or `Xvfb` as CI uses

### Build the app

```bash
cd /path/to/namehold-wallet
pnpm install
pnpm tauri build --debug --no-bundle
```

This is the build CI runs (CI adds `-- --locked`). `wdio.conf.ts` launches `src-tauri/target/debug/namehold-wallet`, so a release build (`pnpm tauri build` without `--debug`) is not picked up, and neither is a binary in a Cargo target directory moved elsewhere by `CARGO_TARGET_DIR` or `build.target-dir`.

### Run E2E tests

```bash
tauri-driver &          # WebDriver on 127.0.0.1:4444; it starts WebKitWebDriver
cd e2e
pnpm install --ignore-workspace
pnpm test
```

The test runner does not start the driver: `tauri-driver` must already listen on port 4444. WebdriverIO then:
1. Asks `tauri-driver` for a session, which launches `../src-tauri/target/debug/namehold-wallet`
2. Runs the specs in `specs/` one at a time (`maxInstances: 1`: the driver serves one app session)
3. Reports results and exits

### Debug a specific test

```bash
cd e2e
pnpm test:debug
```

## CI integration

E2E tests run on GitHub Actions in the job `E2E Tests (Tauri App)` (`.github/workflows/e2e.yml`), on pushes and pull requests to `main`. The runner image is pinned to `ubuntu-26.04`, because the WebDriver package is installed under the name that release gives it (`webkitgtk-webdriver`).

1. Install the Tauri build dependencies plus `webkitgtk-webdriver`, `xvfb`, `gsettings-desktop-schemas`, `dbus-x11` and Mesa's drivers
2. Build the debug binary (`pnpm tauri build --debug --no-bundle -- --locked`)
3. Install E2E dependencies (`pnpm install --frozen-lockfile --ignore-workspace --ignore-scripts --config.minimum-release-age=0` in `e2e/`)
4. Install `tauri-driver` 2.1.0 with Cargo
5. Start `Xvfb`, then `tauri-driver` and `pnpm test` under a private D-Bus session
6. On failure, upload the artifact `e2e-logs`: `e2e/logs/`, `/tmp/tauri-driver.log` and `/tmp/xvfb.log`. When sessions die, read `tauri-driver.log` first.

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
- **Linux only**: `wdio.conf.ts` has no macOS or Windows configuration, and CI runs on Linux
- **App startup time**: Tests use 5–60 second timeouts; adjust `mochaOpts.timeout` in `wdio.conf.ts` if running on slow hardware

## References

- [Tauri WebDriver Example](https://github.com/tauri-apps/webdriver-example)
- [WebdriverIO Docs](https://webdriver.io/)
- [Tauri v2 Testing Guide](https://v2.tauri.app/develop/testing/)
