import type { Settings } from "../types";

/**
 * Settings are persisted as text — the DB stores every value as a string, so a
 * boolean setting is the literal `"true"` or `"false"`. This module is the one
 * place that converts between that on-the-wire form and a real boolean, so UI
 * code isn't littered with `=== "true"` / `? "true" : "false"` rewrapping.
 */

/** Read a persisted boolean. A missing or malformed value reads as `false`. */
export const settingToBool = (value: string | undefined): boolean => value === "true";

/** Write a boolean back in the persisted form. */
export const boolToSetting = (value: boolean): "true" | "false" => (value ? "true" : "false");

/**
 * Secret settings (currently `node_rpc_api_key`) are write-only: the backend
 * never returns the value, and instead reports whether one is stored via a
 * sibling `__has_<key>` marker carrying the same `"true"`/`"false"` text. The
 * markers aren't part of the `Settings` type, hence the cast.
 */
export const hasStoredSecret = (settings: Settings | undefined, key: keyof Settings): boolean =>
  settingToBool((settings as unknown as Record<string, string> | undefined)?.[`__has_${key}`]);
