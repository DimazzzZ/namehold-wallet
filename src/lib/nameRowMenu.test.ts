import { describe, expect, it } from "vitest";
import { nameRowMenu } from "./nameRowMenu";
import type { NameActionCapabilities } from "../types";

const no = { allowed: false, reason: "no" };
const yes = { allowed: true, reason: null };

function caps(over: Partial<NameActionCapabilities> = {}): NameActionCapabilities {
  return {
    name: "atlas",
    phase: "CLOSED",
    taskState: "ownedRegistered",
    ownsName: true,
    nameIsRegistered: true,
    transferPending: false,
    canOpen: no,
    canBid: no,
    canReveal: no,
    canRedeem: no,
    canRegister: no,
    canUpdate: no,
    canTransfer: no,
    canFinalize: no,
    canCancelTransfer: no,
    canRenew: no,
    canRevoke: no,
    ...over,
  } as NameActionCapabilities;
}

const labels = (items: ReturnType<typeof nameRowMenu>) => items.map((i) => i.label);

describe("nameRowMenu", () => {
  it("an owned, registered name: its allowed actions, then its tabs", () => {
    const items = nameRowMenu({
      caps: caps({ canRenew: yes, canTransfer: yes, canUpdate: yes, canRevoke: yes }),
      watched: false,
      watchOnly: false,
    });
    expect(labels(items)).toEqual([
      "Open name",
      "Renew",
      "Transfer or sell…",
      "Edit DNS records",
      "Sign message",
    ]);
    expect(items.find((i) => i.label === "Renew")?.do).toEqual({ kind: "run", action: "renew" });
    expect(items.find((i) => i.label === "Edit DNS records")?.do).toEqual({
      kind: "open",
      tab: "dns",
    });
  });

  it("never offers Revoke: it burns the name and stays on the name page", () => {
    const items = nameRowMenu({ caps: caps({ canRevoke: yes }), watched: false, watchOnly: false });
    expect(labels(items).some((l) => /revoke/i.test(l))).toBe(false);
  });

  it("a name in transfer: Finalize and Cancel transfer when allowed, no Transfer", () => {
    const items = nameRowMenu({
      caps: caps({ transferPending: true, canFinalize: yes, canCancelTransfer: yes }),
      watched: false,
      watchOnly: false,
    });
    expect(labels(items)).toEqual(["Open name", "Finalize", "Cancel transfer…", "Sign message"]);
    expect(items.find((i) => i.label === "Cancel transfer…")?.do).toEqual({ kind: "open" });
  });

  it("an auction position: Reveal and Redeem when allowed, no Sign message", () => {
    const items = nameRowMenu({
      caps: caps({ ownsName: false, nameIsRegistered: false, canReveal: yes, canRedeem: yes }),
      watched: false,
      watchOnly: false,
    });
    expect(labels(items)).toEqual(["Open name", "Reveal", "Redeem"]);
  });

  it("a watched name gets Edit tags; watching itself is the row's star, not the menu", () => {
    const items = nameRowMenu({ caps: undefined, watched: true, watchOnly: false });
    expect(labels(items)).toEqual(["Open name", "Edit tags"]);
  });

  it("no capabilities yet: only Open name", () => {
    expect(labels(nameRowMenu({ caps: undefined, watched: false, watchOnly: false }))).toEqual([
      "Open name",
    ]);
  });

  it("a watch-only wallet signs nothing: no actions and no signing tabs", () => {
    const items = nameRowMenu({
      caps: caps({ canRenew: yes, canTransfer: yes, canUpdate: yes }),
      watched: false,
      watchOnly: true,
    });
    expect(labels(items)).toEqual(["Open name"]);
  });
});
