import { describe, expect, it } from "vitest";
import { yourNamesStatus } from "../YourNamesCard";
import { allowed, makeCapabilities, refused } from "../../test/fixtures/capabilities";
import type { HsdName } from "../../types";

const name = {
  name: "abzanov",
  state: "CLOSED",
  height: 1,
  renewal: 2,
  owner: { hash: "t", index: 0 },
  stats: null,
} as HsdName;

describe("Your names: a name with a transfer pending", () => {
  it("waits during the lockup instead of asking to finalize", () => {
    const c = makeCapabilities({
      name: "abzanov",
      taskState: "transferPendingFinalize",
      canFinalize: refused("lockup"),
      countdownBlocks: 282,
      countdownHours: 47,
    });
    const s = yourNamesStatus(name, c, "mainnet");
    expect(s.main).toBe("Transferring");
    expect(s.suffix).toMatch(/^finalize in /);
    expect(`${s.main} ${s.suffix}`).not.toMatch(/ready|finalize it/i);
  });

  it("says when it is ready to finalize", () => {
    const c = makeCapabilities({
      name: "abzanov",
      taskState: "transferPendingFinalize",
      canFinalize: allowed,
    });
    expect(yourNamesStatus(name, c, "mainnet")).toEqual({
      main: "Transferring",
      suffix: "ready to finalize",
    });
  });
});
