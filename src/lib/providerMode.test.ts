import { describe, it, expect } from "vitest";
import {
  canWrite,
  writeBlockedReason,
  canManageNode,
  isExternalProvider,
  hasWallet,
  isFallbackActive,
  providerTone,
  providerStatusValue,
  parseStringArraySetting,
  serializeStringArraySetting,
  normalizeTransaction,
  totalBalanceDoos,
  providerLabel,
  resolveWriteReason,
} from "./providerMode";
import type {
  ConnectionMode,
  ProviderStatus,
  ReadContext,
  ReadProviderKind,
} from "../types";

function makeProvider(
  overrides: Partial<ProviderStatus> = {},
): ProviderStatus {
  return {
    kind: "local_hsd" as ReadProviderKind,
    label: "Local hsd",
    healthy: true,
    writeCapable: true,
    manageable: true,
    ...overrides,
  };
}

function makeContext(overrides: Partial<ReadContext> = {}): ReadContext {
  return {
    connectionMode: "local_managed_hsd" as ConnectionMode,
    activeReadProvider: makeProvider(),
    fallbackActive: false,
    localNodeHealthy: true,
    walletAvailable: true,
    writeAllowed: true,
    ...overrides,
  };
}

describe("providerLabel", () => {
  it("uses the canonical label for local hsd", () => {
    expect(providerLabel("local_hsd")).toBe("Local hsd");
  });

  it("uses the canonical label for the external explorer", () => {
    expect(providerLabel("external_hnsfans")).toBe("HNSFans");
  });

  it("falls back to the canonical label for remote without a custom name", () => {
    expect(providerLabel("remote_hsd")).toBe("Remote hsd");
    expect(providerLabel("remote_hsd", null)).toBe("Remote hsd");
    expect(providerLabel("remote_hsd", "   ")).toBe("Remote hsd");
  });

  it("prefers a trimmed custom label for remote hsd", () => {
    expect(providerLabel("remote_hsd", "  Home server  ")).toBe("Home server");
  });

  it("ignores custom labels for non-remote providers", () => {
    expect(providerLabel("local_hsd", "Ignored")).toBe("Local hsd");
  });
});

describe("resolveWriteReason", () => {
  it("blocks when write mode is disabled, regardless of context", () => {
    expect(resolveWriteReason(makeContext({ writeAllowed: true }), false)).toBe(
      "Write mode is disabled. Enable it in Settings to send or transfer.",
    );
  });

  it("returns null when write mode is on and context allows writes", () => {
    expect(
      resolveWriteReason(makeContext({ writeAllowed: true }), true),
    ).toBeNull();
  });

  it("defers to the provider reason when write mode is on but context blocks", () => {
    const ctx = makeContext({
      writeAllowed: false,
      writeReason: "Remote node is untrusted.",
    });
    expect(resolveWriteReason(ctx, true)).toBe("Remote node is untrusted.");
  });

  it("explains unknown context when write mode is on", () => {
    expect(resolveWriteReason(null, true)).toBe("Provider status is unknown.");
  });
});

describe("canWrite", () => {
  it("is false for null/undefined context", () => {
    expect(canWrite(null)).toBe(false);
    expect(canWrite(undefined)).toBe(false);
  });

  it("reflects writeAllowed", () => {
    expect(canWrite(makeContext({ writeAllowed: true }))).toBe(true);
    expect(canWrite(makeContext({ writeAllowed: false }))).toBe(false);
  });
});

describe("writeBlockedReason", () => {
  it("explains unknown context", () => {
    expect(writeBlockedReason(null)).toBe("Provider status is unknown.");
  });

  it("returns null when writes are allowed", () => {
    expect(writeBlockedReason(makeContext({ writeAllowed: true }))).toBeNull();
  });

  it("prefers explicit writeReason", () => {
    const ctx = makeContext({
      writeAllowed: false,
      writeReason: "Remote node is untrusted.",
    });
    expect(writeBlockedReason(ctx)).toBe("Remote node is untrusted.");
  });

  it("falls back to provider reason", () => {
    const ctx = makeContext({
      writeAllowed: false,
      writeReason: null,
      activeReadProvider: makeProvider({ reason: "Provider is read-only." }),
    });
    expect(writeBlockedReason(ctx)).toBe("Provider is read-only.");
  });

  it("falls back to a generic message", () => {
    const ctx = makeContext({
      writeAllowed: false,
      writeReason: null,
      activeReadProvider: makeProvider({ reason: undefined }),
    });
    expect(writeBlockedReason(ctx)).toBe(
      "Writes are not available in the current mode.",
    );
  });
});

describe("canManageNode", () => {
  it("is false without context", () => {
    expect(canManageNode(null)).toBe(false);
  });

  it("reflects provider manageable flag", () => {
    expect(
      canManageNode(
        makeContext({ activeReadProvider: makeProvider({ manageable: true }) }),
      ),
    ).toBe(true);
    expect(
      canManageNode(
        makeContext({
          activeReadProvider: makeProvider({ manageable: false }),
        }),
      ),
    ).toBe(false);
  });
});

describe("isExternalProvider", () => {
  it("detects the external explorer provider", () => {
    expect(
      isExternalProvider(
        makeContext({
          activeReadProvider: makeProvider({ kind: "external_hnsfans" }),
        }),
      ),
    ).toBe(true);
  });

  it("is false for local/remote hsd", () => {
    expect(isExternalProvider(makeContext())).toBe(false);
    expect(
      isExternalProvider(
        makeContext({
          activeReadProvider: makeProvider({ kind: "remote_hsd" }),
        }),
      ),
    ).toBe(false);
  });
});

describe("hasWallet / isFallbackActive", () => {
  it("reflects walletAvailable", () => {
    expect(hasWallet(makeContext({ walletAvailable: true }))).toBe(true);
    expect(hasWallet(makeContext({ walletAvailable: false }))).toBe(false);
  });

  it("reflects fallbackActive", () => {
    expect(isFallbackActive(makeContext({ fallbackActive: true }))).toBe(true);
    expect(isFallbackActive(makeContext({ fallbackActive: false }))).toBe(
      false,
    );
  });
});

describe("providerTone", () => {
  it("is default without context", () => {
    expect(providerTone(null)).toBe("default");
  });

  it("is error when provider is unhealthy", () => {
    expect(
      providerTone(
        makeContext({ activeReadProvider: makeProvider({ healthy: false }) }),
      ),
    ).toBe("error");
  });

  it("is warning on fallback", () => {
    expect(providerTone(makeContext({ fallbackActive: true }))).toBe("warning");
  });

  it("is warning for external provider", () => {
    expect(
      providerTone(
        makeContext({
          activeReadProvider: makeProvider({ kind: "external_hnsfans" }),
        }),
      ),
    ).toBe("warning");
  });

  it("is success for a healthy local provider", () => {
    expect(providerTone(makeContext())).toBe("success");
  });
});

describe("providerStatusValue", () => {
  it("is Unknown without context", () => {
    expect(providerStatusValue(null)).toBe("Unknown");
  });

  it("is Unavailable when unhealthy", () => {
    expect(
      providerStatusValue(
        makeContext({ activeReadProvider: makeProvider({ healthy: false }) }),
      ),
    ).toBe("Unavailable");
  });

  it("notes fallback read-only", () => {
    expect(providerStatusValue(makeContext({ fallbackActive: true }))).toBe(
      "Read-only (fallback)",
    );
  });

  it("is Read-only for external provider", () => {
    expect(
      providerStatusValue(
        makeContext({
          activeReadProvider: makeProvider({ kind: "external_hnsfans" }),
        }),
      ),
    ).toBe("Read-only");
  });

  it("is Connected for a healthy local provider", () => {
    expect(providerStatusValue(makeContext())).toBe("Connected");
  });
});

describe("parseStringArraySetting", () => {
  it("returns [] for empty/null", () => {
    expect(parseStringArraySetting(null)).toEqual([]);
    expect(parseStringArraySetting(undefined)).toEqual([]);
    expect(parseStringArraySetting("")).toEqual([]);
  });

  it("parses a JSON string array, trimming and filtering", () => {
    expect(parseStringArraySetting('[" a ", "b", "", "  "]')).toEqual([
      "a",
      "b",
    ]);
  });

  it("returns [] for non-array JSON", () => {
    expect(parseStringArraySetting('{"a":1}')).toEqual([]);
  });

  it("returns [] for malformed JSON", () => {
    expect(parseStringArraySetting("not json")).toEqual([]);
  });
});

describe("serializeStringArraySetting", () => {
  it("trims and filters before serializing", () => {
    expect(serializeStringArraySetting([" a ", "", "b"])).toBe('["a","b"]');
  });

  it("round-trips with the parser", () => {
    const values = ["addr1", "addr2"];
    expect(
      parseStringArraySetting(serializeStringArraySetting(values)),
    ).toEqual(values);
  });
});

describe("normalizeTransaction", () => {
  it("normalizes an hsd-style tx with outputs", () => {
    const row = normalizeTransaction(
      {
        hash: "abc",
        confirmations: 3,
        height: 100,
        fee: 1000,
        outputs: [
          { value: 500_000, address: "hs1qaddr" },
          { value: 250_000, address: "" },
        ],
      },
      0,
    );
    expect(row.hash).toBe("abc");
    expect(row.confirmed).toBe(true);
    expect(row.confirmations).toBe(3);
    expect(row.height).toBe(100);
    expect(row.amountDoos).toBe(750_000);
    expect(row.amountHns).toBeCloseTo(0.75);
    expect(row.address).toBe("hs1qaddr");
    expect(row.direction).toBe("send");
    expect(row.tone).toBe("success");
  });

  it("treats zero-fee output tx as receive", () => {
    const row = normalizeTransaction(
      { txid: "def", outputs: [{ value: 100, address: "x" }] },
      1,
    );
    expect(row.hash).toBe("def");
    expect(row.direction).toBe("receive");
  });

  it("normalizes a flat external tx (direction hint)", () => {
    const row = normalizeTransaction(
      {
        txid: "ext1",
        value: 1_000_000,
        direction: "OUT",
        address: "hs1qext",
        confirmed: false,
        matchedAddress: "hs1qmatch",
      },
      2,
    );
    expect(row.hash).toBe("ext1");
    expect(row.direction).toBe("send");
    expect(row.amountDoos).toBe(1_000_000);
    expect(row.address).toBe("hs1qmatch");
    expect(row.confirmed).toBe(false);
    expect(row.tone).toBe("warning");
  });

  it("infers direction from sign when no hint", () => {
    const sent = normalizeTransaction({ amount: -42 }, 3);
    expect(sent.direction).toBe("send");
    expect(sent.amountDoos).toBe(42);

    const received = normalizeTransaction({ amount: 42 }, 4);
    expect(received.direction).toBe("receive");
  });

  it("synthesizes a hash when none provided", () => {
    const row = normalizeTransaction({ value: 1 }, 7);
    expect(row.hash).toBe("tx-7");
  });

  it("derives timestamp from mtime/time/timestamp", () => {
    expect(normalizeTransaction({ mtime: 1_700_000_000 }, 0).timestamp).toBe(
      new Date(1_700_000_000 * 1000).toISOString(),
    );
    expect(normalizeTransaction({ time: 1_700_000_000 }, 0).timestamp).toBe(
      new Date(1_700_000_000 * 1000).toISOString(),
    );
    expect(
      normalizeTransaction({ timestamp: "2024-01-01T00:00:00Z" }, 0).timestamp,
    ).toBe("2024-01-01T00:00:00Z");
  });
});

describe("totalBalanceDoos", () => {
  it("is 0 for null/undefined", () => {
    expect(totalBalanceDoos(null)).toBe(0);
    expect(totalBalanceDoos(undefined)).toBe(0);
  });

  it("sums confirmed and unconfirmed", () => {
    expect(
      totalBalanceDoos({
        confirmed: 1_000_000,
        unconfirmed: 500_000,
        locked_confirmed: null,
        locked_unconfirmed: null,
      }),
    ).toBe(1_500_000);
  });
});
