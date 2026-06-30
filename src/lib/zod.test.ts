import { describe, it, expect } from "vitest";
import {
  MigrationStatus,
  BatchStatus,
  AssetSchema,
  BatchSchema,
  HsdBalanceSchema,
} from "./zod";

describe("MigrationStatus", () => {
  it("accepts all valid statuses", () => {
    const valid = [
      "not_started",
      "namebase_transfer_requested",
      "waiting_transfer_tx",
      "transfer_seen_on_chain",
      "waiting_finalize",
      "finalized_owned",
      "failed_or_stuck",
      "do_not_touch_staked",
    ];
    valid.forEach((s) => {
      expect(MigrationStatus.parse(s)).toBe(s);
    });
  });

  it("rejects invalid status", () => {
    expect(() => MigrationStatus.parse("invalid")).toThrow();
    expect(() => MigrationStatus.parse("")).toThrow();
    expect(() => MigrationStatus.parse("PENDING")).toThrow();
  });
});

describe("BatchStatus", () => {
  it("accepts all valid statuses", () => {
    const valid = ["planned", "in_progress", "completed", "paused", "cancelled"];
    valid.forEach((s) => {
      expect(BatchStatus.parse(s)).toBe(s);
    });
  });

  it("rejects invalid status", () => {
    expect(() => BatchStatus.parse("invalid")).toThrow();
    expect(() => BatchStatus.parse("active")).toThrow();
  });
});

describe("AssetSchema", () => {
  const validAsset = {
    id: 1,
    tld: "crypto",
    status: "not_started",
    is_staked: false,
    category: "Premium",
    tags: ["high_value"],
    notes: "test note",
    hns_received: 1000000,
    transfer_tx_hash: null,
    finalize_tx_hash: null,
    name_state: null,
    expires_at_height: null,
    days_until_expire: null,
    last_synced_at: null,
    created_at: "2024-01-01T00:00:00",
    updated_at: "2024-01-01T00:00:00",
  };

  it("accepts valid asset", () => {
    expect(AssetSchema.safeParse(validAsset).success).toBe(true);
  });

  it("accepts with all nulls", () => {
    const result = AssetSchema.safeParse({
      ...validAsset,
      category: null,
      notes: null,
      hns_received: null,
      tags: [],
    });
    expect(result.success).toBe(true);
  });

  it("rejects missing required field", () => {
    const { tld, ...noTld } = validAsset;
    expect(AssetSchema.safeParse(noTld).success).toBe(false);
  });

  it("rejects invalid status", () => {
    expect(AssetSchema.safeParse({ ...validAsset, status: "bad" }).success).toBe(false);
  });

  it("rejects wrong type for is_staked", () => {
    expect(AssetSchema.safeParse({ ...validAsset, is_staked: "yes" }).success).toBe(false);
  });

  it("rejects wrong type for id", () => {
    expect(AssetSchema.safeParse({ ...validAsset, id: "one" }).success).toBe(false);
  });
});

describe("BatchSchema", () => {
  it("accepts valid batch", () => {
    expect(
      BatchSchema.safeParse({
        id: 1,
        name: "Test",
        description: "desc",
        status: "planned",
        asset_count: 5,
        created_at: "2024-01-01",
        updated_at: "2024-01-01",
      }).success
    ).toBe(true);
  });

  it("accepts null description", () => {
    expect(
      BatchSchema.safeParse({
        id: 1,
        name: "Test",
        description: null,
        status: "completed",
        asset_count: null,
        created_at: "2024-01-01",
        updated_at: "2024-01-01",
      }).success
    ).toBe(true);
  });

  it("rejects invalid status", () => {
    expect(
      BatchSchema.safeParse({
        id: 1,
        name: "Test",
        description: null,
        status: "invalid",
        asset_count: 0,
        created_at: "2024-01-01",
        updated_at: "2024-01-01",
      }).success
    ).toBe(false);
  });
});

describe("HsdBalanceSchema", () => {
  it("accepts valid balance", () => {
    expect(
      HsdBalanceSchema.safeParse({
        confirmed: 1000000,
        unconfirmed: 500000,
        locked_unconfirmed: 0,
        locked_confirmed: 0,
      }).success
    ).toBe(true);
  });

  it("accepts null locked fields", () => {
    expect(
      HsdBalanceSchema.safeParse({
        confirmed: 1000000,
        unconfirmed: 0,
        locked_unconfirmed: null,
        locked_confirmed: null,
      }).success
    ).toBe(true);
  });
});
