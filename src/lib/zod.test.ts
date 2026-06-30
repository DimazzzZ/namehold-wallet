import { describe, it, expect } from "vitest";
import { AssetSchema, BatchSchema, HsdBalanceSchema, MigrationStatus, BatchStatus } from "./zod";

describe("MigrationStatus", () => {
  it("accepts valid statuses", () => {
    expect(MigrationStatus.parse("not_started")).toBe("not_started");
    expect(MigrationStatus.parse("finalized_owned")).toBe("finalized_owned");
    expect(MigrationStatus.parse("do_not_touch_staked")).toBe("do_not_touch_staked");
  });
  it("rejects invalid status", () => {
    expect(() => MigrationStatus.parse("invalid")).toThrow();
  });
});

describe("BatchStatus", () => {
  it("accepts valid statuses", () => {
    expect(BatchStatus.parse("planned")).toBe("planned");
    expect(BatchStatus.parse("in_progress")).toBe("in_progress");
    expect(BatchStatus.parse("completed")).toBe("completed");
  });
  it("rejects invalid status", () => {
    expect(() => BatchStatus.parse("invalid")).toThrow();
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
    const result = AssetSchema.safeParse(validAsset);
    expect(result.success).toBe(true);
  });
  it("accepts asset with nulls", () => {
    const result = AssetSchema.safeParse({ ...validAsset, category: null, notes: null, hns_received: null });
    expect(result.success).toBe(true);
  });
  it("rejects missing required field", () => {
    const { tld, ...noTld } = validAsset;
    const result = AssetSchema.safeParse(noTld);
    expect(result.success).toBe(false);
  });
  it("rejects invalid status", () => {
    const result = AssetSchema.safeParse({ ...validAsset, status: "invalid" });
    expect(result.success).toBe(false);
  });
  it("rejects wrong type for is_staked", () => {
    const result = AssetSchema.safeParse({ ...validAsset, is_staked: "yes" });
    expect(result.success).toBe(false);
  });
});

describe("BatchSchema", () => {
  it("accepts valid batch", () => {
    const result = BatchSchema.safeParse({
      id: 1,
      name: "Test Batch",
      description: "desc",
      status: "planned",
      asset_count: 5,
      created_at: "2024-01-01",
      updated_at: "2024-01-01",
    });
    expect(result.success).toBe(true);
  });
  it("accepts null description", () => {
    const result = BatchSchema.safeParse({
      id: 1,
      name: "Test",
      description: null,
      status: "completed",
      asset_count: null,
      created_at: "2024-01-01",
      updated_at: "2024-01-01",
    });
    expect(result.success).toBe(true);
  });
  it("rejects invalid status", () => {
    const result = BatchSchema.safeParse({
      id: 1,
      name: "Test",
      description: null,
      status: "invalid",
      asset_count: 0,
      created_at: "2024-01-01",
      updated_at: "2024-01-01",
    });
    expect(result.success).toBe(false);
  });
});

describe("HsdBalanceSchema", () => {
  it("accepts valid balance", () => {
    const result = HsdBalanceSchema.safeParse({
      confirmed: 1000000,
      unconfirmed: 500000,
      locked_unconfirmed: 0,
      locked_confirmed: 0,
    });
    expect(result.success).toBe(true);
  });
  it("accepts null locked fields", () => {
    const result = HsdBalanceSchema.safeParse({
      confirmed: 1000000,
      unconfirmed: 0,
      locked_unconfirmed: null,
      locked_confirmed: null,
    });
    expect(result.success).toBe(true);
  });
});
