/**
 * WebdriverIO config for Tauri E2E tests.
 * Tests cover runtime-only code paths: app lifecycle, window management, secure prompts.
 *
 * Prerequisites (see e2e/README.md and .github/workflows/e2e.yml):
 *   1. Build the app:   pnpm tauri build   (from repo root)
 *   2. Start the driver: tauri-driver       (Cargo binary: `cargo install tauri-driver`)
 *      — listens on port 4444 and spawns WebKitWebDriver under the hood.
 *   3. Run tests:        pnpm test          (from e2e/)
 */

import path from "node:path";
import { fileURLToPath } from "node:url";

// The built executable. Tauri v2 names the release EXECUTABLE after the Cargo
// package name ("namehold-wallet") — `productName` ("Namehold") only names the
// bundles (.deb/.rpm/.AppImage), which `tauri build --no-bundle` skips.
// E2E builds with `tauri build --debug --no-bundle` (smoke tests don't need
// optimization), so the binary is in target/debug/. Confirmed from CI:
//   "Built application at: .../target/debug/namehold-wallet".
//
// Resolve to an ABSOLUTE path: tauri-driver launches the binary relative to
// its OWN working directory, not e2e/, so a relative path fails to spawn the
// app (the session then dies with connection-refused).
const configDir = path.dirname(fileURLToPath(import.meta.url));
const appBinary = path.resolve(configDir, "../src-tauri/target/debug/namehold-wallet");

export const config: WebdriverIO.Config = {
  runner: "local",
  // Connect directly to tauri-driver — no /wd/hub path prefix.
  hostname: "127.0.0.1",
  port: 4444,
  path: "/",
  specs: ["./specs/**/*.e2e.ts"],
  // tauri-driver proxies to a SINGLE WebKitWebDriver/app session at a time.
  // WDIO otherwise launches one worker per spec in parallel, so multiple app
  // instances race for the one driver session — the sockets reset and every
  // "Failed to create a session: UND_ERR_SOCKET". Force strict serial runs.
  maxInstances: 1,
  framework: "mocha",
  mochaOpts: {
    timeout: 60000,
    ui: "bdd",
  },
  reporters: ["spec"],

  // Tauri app configuration.
  // The app must be built before tests run: `pnpm tauri build`.
  // The driver must be running on port 4444 before `pnpm test` is invoked.
  capabilities: [
    {
      platformName: "linux",
      // WDIO 9 defaults to opening a WebDriver BiDi session (it appends
      // webSocketUrl: true to the capabilities). tauri-driver + WebKitWebDriver
      // only speak classic W3C WebDriver, so the BiDi handshake closes the
      // socket mid-response — surfacing as "Failed to create a session:
      // UND_ERR_SOCKET". Force classic mode to negotiate a plain W3C session.
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": {
        application: appBinary,
      },
    } as WebdriverIO.Capabilities,
  ],

  // Hook: log test start/end.
  beforeTest: (test) => {
    console.log(`\n[TEST] ${test.title}`);
  },

  afterTest: (test, context, { passed }) => {
    if (passed) {
      console.log(`[PASS] ${test.title}`);
    } else {
      console.log(`[FAIL] ${test.title}`);
    }
  },
};
