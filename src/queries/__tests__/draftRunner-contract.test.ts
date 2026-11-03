import { describe, expect, it } from "vitest";

// A built draft reserves its coins, so every flow that builds one must end it
// through useDraftRunner (send, settleFailed, abandon), whose rule is tested in
// draftRunner.test.tsx. A flow that signs or broadcasts with the raw hooks
// decides a draft's fate on its own, and two such flows (Send, a batch's
// Cancel) once left coins reserved until "not enough funds". This test keeps
// the raw hooks to the places allowed below, each with its reason.

// A call, not the declaration (`export function useSignTxDraft()`).
const RAW =
  /(?<!function )\b(useExecuteDraft|useSignTxDraft|useBroadcastTxDraft|useDeleteTxDraft)\(\)/g;

const ALLOWED: Record<string, { hooks: string[]; why: string }> = {
  "src/queries/wallet.ts": {
    hooks: ["useSignTxDraft", "useBroadcastTxDraft"],
    why: "useExecuteDraft, the step the runner wraps, is built from them",
  },
  "src/queries/useDraftRunner.ts": {
    hooks: ["useExecuteDraft", "useDeleteTxDraft"],
    why: "the runner itself",
  },
  "src/components/activity/ToFinishList.tsx": {
    hooks: ["useExecuteDraft", "useDeleteTxDraft"],
    why: "Activity rows are drafts that outlive any dialog: Sign & broadcast keeps the row on failure, and Discard is the user's explicit command",
  },
  "src/components/market/MyListings.tsx": {
    hooks: ["useBroadcastTxDraft", "useDeleteTxDraft"],
    why: "a listing's signed draft is kept by design (Send again or Discard on its row); the explicit Discard deletes it",
  },
  "src/components/market/FinalizeAndSign.tsx": {
    hooks: ["useBroadcastTxDraft"],
    why: "the backend signs and keeps the listing's draft; only its broadcast is sent here, and a failure leaves it on the My listings row",
  },
};

// Every source file of the app, read as text at test time (tests excluded).
const SOURCES = import.meta.glob<string>(
  ["/src/**/*.{ts,tsx}", "!/src/**/__tests__/**", "!/src/**/*.test.{ts,tsx}"],
  { query: "?raw", import: "default", eager: true },
);

describe("every flow that builds a draft ends it through useDraftRunner", () => {
  const uses = Object.entries(SOURCES)
    .map(([path, text]) => ({
      file: path.slice(1),
      hooks: [...text.matchAll(RAW)].map((m) => m[1]!),
    }))
    .filter((u) => u.hooks.length > 0);

  it("no other file signs, broadcasts or discards a draft with the raw hooks", () => {
    const offenders = uses
      .map((u) => ({ ...u, extra: u.hooks.filter((h) => !ALLOWED[u.file]?.hooks.includes(h)) }))
      .filter((u) => u.extra.length > 0)
      .map((u) => `${u.file}: ${u.extra.join(", ")}`);
    expect(offenders).toEqual([]);
  });

  it("every allowance is still needed (no stale entries)", () => {
    const stale = Object.entries(ALLOWED).flatMap(([file, a]) =>
      a.hooks
        .filter((h) => !uses.find((u) => u.file === file)?.hooks.includes(h))
        .map((h) => `${file}: ${h}`),
    );
    expect(stale).toEqual([]);
  });
});
