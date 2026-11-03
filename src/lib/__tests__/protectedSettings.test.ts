import { describe, it, expect } from "vitest";
import securitySource from "../../../src-tauri/src/security.rs?raw";
import errorSource from "../../../src-tauri/src/error.rs?raw";
import {
  CONFIRMATION_DECLINED,
  PROTECTED_SETTING_KEYS,
  isConfirmationDeclined,
  isProtectedSettingKey,
  sessionTimeoutError,
} from "../protectedSettings";
import { DEFAULT_SETTINGS } from "../settingsDefaults";
import { networkNodeSettingKey } from "../nodeNetwork";

/** The string items of the `pub const <name>: &[&str] = &[...]` list in security.rs. */
function rustList(name: string): string[] {
  const m = securitySource.match(
    new RegExp(`pub const ${name}: &\\[&str\\] = &\\[([\\s\\S]*?)\\];`),
  );
  if (!m) throw new Error(`${name} not found in security.rs`);
  return [...m[1]!.matchAll(/"([^"]+)"/g)].map((x) => x[1]!);
}

describe("protected settings (contract with src-tauri/src/security.rs)", () => {
  it("holds exactly the backend's PROTECTED_SETTING_KEYS", () => {
    const rust = rustList("PROTECTED_SETTING_KEYS");
    expect(rust.length).toBeGreaterThan(0);
    expect([...PROTECTED_SETTING_KEYS].sort()).toEqual([...rust].sort());
  });

  it("every setting a screen saves is one the backend takes from the renderer", () => {
    const accepted = new Set([
      ...rustList("RENDERER_SETTING_KEYS"),
      ...rustList("PROTECTED_SETTING_KEYS"),
    ]);
    // Written by the tray backend when it shows its hint, never by a screen.
    const backendOnly = new Set(["tray_hint_shown"]);
    for (const key of Object.keys(DEFAULT_SETTINGS)) {
      if (backendOnly.has(key)) continue;
      expect(accepted.has(key), `${key} would be refused by update_setting`).toBe(true);
    }
    for (const network of ["mainnet", "testnet", "regtest"]) {
      for (const base of ["node_mode", "autostart_hsd"] as const) {
        const key = networkNodeSettingKey(base, network);
        expect(accepted.has(key), key).toBe(true);
      }
    }
  });

  it("knows a protected key from a plain one", () => {
    expect(isProtectedSettingKey("hsd_path")).toBe(true);
    expect(isProtectedSettingKey("node_mode.main")).toBe(true);
    expect(isProtectedSettingKey("autostart_hsd.main")).toBe(false);
    expect(isProtectedSettingKey("close_to_tray")).toBe(false);
  });
});

describe("a declined confirmation", () => {
  it("is known by the words of AppError::UserRejected", () => {
    expect(errorSource).toMatch(
      new RegExp(`#\\[error\\("${CONFIRMATION_DECLINED}"\\)\\]\\s*UserRejected,`),
    );
  });

  it("is that error and nothing else", () => {
    expect(isConfirmationDeclined("Confirmation declined")).toBe(true);
    expect(isConfirmationDeclined(new Error("Confirmation declined"))).toBe(true);
    expect(isConfirmationDeclined("Invalid input: Confirmation declined")).toBe(false);
    expect(isConfirmationDeclined("Wallet locked")).toBe(false);
    expect(isConfirmationDeclined(undefined)).toBe(false);
  });
});

describe("sessionTimeoutError (the backend's rule)", () => {
  it("takes empty or a whole number of seconds from 60 to 3600", () => {
    for (const ok of ["", "  ", "60", "900", "3600"]) {
      expect(sessionTimeoutError(ok), ok).toBeNull();
    }
    for (const bad of ["0", "59", "3601", "-5", "1.5", "abc", "18446744073709551616"]) {
      expect(sessionTimeoutError(bad), bad).toMatch(/whole number of seconds/);
    }
  });
});
