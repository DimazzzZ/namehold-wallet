import { DEFAULT_SETTINGS, useSettingsStore } from "../../stores/settings";
import type { Settings } from "../../types";

/**
 * A full `Settings` object for component tests, built on the store's own
 * DEFAULT_SETTINGS so adding a setting never requires touching every test —
 * a test overrides only the keys it cares about. `onboarding_complete` is
 * forced to "true" because almost every screen under test assumes a finished
 * onboarding; the onboarding tests override it back.
 *
 * `over` is a loose string map (not `Partial<Settings>`) so tests can also set
 * backend-only markers like `__has_node_rpc_api_key`.
 */
export function makeSettings(over: Record<string, string> = {}): Settings {
  return { ...DEFAULT_SETTINGS, onboarding_complete: "true", ...over } as Settings;
}

/** Seed the zustand settings store as if `load()` had already run. */
export function loadSettings(over: Record<string, string> = {}): void {
  useSettingsStore.setState({ loaded: true, settings: makeSettings(over) });
}
