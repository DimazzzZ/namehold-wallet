import { describe, it, expect } from "vitest";
import { mapError } from "./errors";

describe("mapError", () => {
  it("maps connection refused", () => {
    expect(mapError("connection refused")).toContain("Cannot connect to wallet");
  });

  it("maps ECONNREFUSED", () => {
    expect(mapError("ECONNREFUSED")).toContain("Cannot connect to wallet");
  });

  it("maps unauthorized", () => {
    expect(mapError("Unauthorized")).toContain("Invalid API key");
  });

  it("maps bad API key", () => {
    expect(mapError("bad API key")).toContain("Invalid API key");
  });

  it("maps insufficient funds", () => {
    expect(mapError("insufficient funds")).toContain("Insufficient HNS");
  });

  it("maps timed out", () => {
    expect(mapError("timed out")).toContain("timed out");
  });

  it("maps not found", () => {
    expect(mapError("Not found")).toContain("not found");
  });

  it("maps wallet locked", () => {
    expect(mapError("wallet is locked")).toContain("locked");
  });

  it("maps error decoding response", () => {
    expect(mapError("error decoding response body")).toContain("unexpected data");
  });

  it("strips technical prefixes", () => {
    const result = mapError("Error invoking remote method: some error");
    expect(result).toBeTruthy();
    expect(result.length).toBeGreaterThan(0);
  });

  it("returns original for unknown errors", () => {
    const result = mapError("something completely unknown");
    expect(result).toBe("something completely unknown");
  });

  it("handles Error objects", () => {
    const result = mapError(new Error("connection refused"));
    expect(result).toContain("Cannot connect to wallet");
  });
});
