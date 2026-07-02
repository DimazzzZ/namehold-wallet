/**
 * Global vitest setup.
 *
 * The app reaches the Rust backend through a single seam, `src/lib/invoke.ts`,
 * which decides at runtime whether to call the real `@tauri-apps/api/core`
 * invoke (desktop shell) or the static web-QA mock (plain browser). It picks
 * the browser path whenever `window.__TAURI_INTERNALS__` is absent.
 *
 * Under jsdom, `window` exists but `__TAURI_INTERNALS__` does not, so every
 * `invoke()` call would be silently routed to the static web-QA mock —
 * bypassing each test's own `vi.mock("@tauri-apps/api/core", ...)` and making
 * scenario-driven assertions impossible.
 *
 * Marking the environment as a Tauri shell makes the wrapper delegate to the
 * (per-test mocked) `@tauri-apps/api/core` invoke, so each test's mock actually
 * drives the data the components see.
 *
 * KNOWN LIMITATION: this exercises the frontend against hand-written backend
 * response shapes; it does NOT verify those shapes match the real Rust
 * backend. Contract drift (a renamed/retyped field on the Rust side) will pass
 * silently here. That gap is covered separately by the Rust⇄TS contract test
 * (see src/lib/__tests__/backend-contract.test.ts) and the regtest live-node
 * suite.
 */
import "@testing-library/jest-dom";

if (typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window)) {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    value: {},
    configurable: true,
    writable: true,
  });
}
