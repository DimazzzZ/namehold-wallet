import { describe, it, expect } from "vitest";
import { settingToBool, boolToSetting, hasStoredSecret } from "../settingsBool";
import type { Settings } from "../../types";

describe("settingToBool", () => {
  it("reads the persisted true", () => {
    expect(settingToBool("true")).toBe(true);
  });

  it("reads anything else as false", () => {
    expect(settingToBool("false")).toBe(false);
    expect(settingToBool("")).toBe(false);
    expect(settingToBool("TRUE")).toBe(false);
    expect(settingToBool("1")).toBe(false);
  });

  it("reads a missing setting as false rather than throwing", () => {
    expect(settingToBool(undefined)).toBe(false);
  });
});

describe("boolToSetting", () => {
  it("writes the persisted text form", () => {
    expect(boolToSetting(true)).toBe("true");
    expect(boolToSetting(false)).toBe("false");
  });

  it("round-trips through settingToBool", () => {
    expect(settingToBool(boolToSetting(true))).toBe(true);
    expect(settingToBool(boolToSetting(false))).toBe(false);
  });
});

describe("hasStoredSecret", () => {
  const withMarker = (value: string) =>
    ({ __has_node_rpc_api_key: value }) as unknown as Settings;

  it("reads the __has_<key> marker the backend sends alongside a write-only secret", () => {
    expect(hasStoredSecret(withMarker("true"), "node_rpc_api_key")).toBe(true);
    expect(hasStoredSecret(withMarker("false"), "node_rpc_api_key")).toBe(false);
  });

  it("reports no stored secret when the marker is absent", () => {
    expect(hasStoredSecret({} as Settings, "node_rpc_api_key")).toBe(false);
  });

  it("reports no stored secret when settings haven't loaded yet", () => {
    expect(hasStoredSecret(undefined, "node_rpc_api_key")).toBe(false);
  });
});
