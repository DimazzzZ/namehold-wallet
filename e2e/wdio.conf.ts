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

// The built binary, relative to this config's directory (e2e/). Tauri v2 names
// the release binary after `productName` ("Namehold") when `mainBinaryName` is
// unset — NOT after the Cargo package name ("namehold-wallet").
const appBinary = "../src-tauri/target/release/Namehold";

export const config: WebdriverIO.Config = {
  runner: "local",
  port: 4444,
  specs: ["./specs/**/*.e2e.ts"],
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
