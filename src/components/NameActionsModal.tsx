import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  useActiveProfile,
  useSignerSession,
  useWriteCapability,
  useNameAction,
  useExecuteDraft,
} from "../queries/wallet";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { Dialog } from "./ui/Dialog";
import { useUiStore } from "../stores/ui";
import { mapError } from "../lib/errors";

/**
 * One modal that exposes every name covenant action for a single name, wired to
 * the `build_*_draft` commands + the build→unlock→sign→broadcast runner.
 *
 * Records (REGISTER/UPDATE) are entered as a JSON array, e.g.
 * `[{"type":"TXT","txt":["hello"]}]` — functional for testing; a richer editor
 * is a later polish pass.
 */
export function NameActionsModal({
  name,
  open,
  onClose,
}: {
  name: string;
  open: boolean;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const { data: signer } = useSignerSession();
  const { data: writeCap } = useWriteCapability();
  const exec = useExecuteDraft();

  const build = {
    open: useNameAction("build_open_draft"),
    bid: useNameAction("build_bid_draft"),
    reveal: useNameAction("build_reveal_draft"),
    redeem: useNameAction("build_redeem_draft"),
    register: useNameAction("build_register_draft"),
    update: useNameAction("build_update_draft"),
    renew: useNameAction("build_renew_draft"),
    transfer: useNameAction("build_transfer_draft"),
    finalize: useNameAction("build_finalize_draft"),
    cancel: useNameAction("build_cancel_draft"),
    revoke: useNameAction("build_revoke_draft"),
  };

  const [bidValue, setBidValue] = useState("");
  const [lockup, setLockup] = useState("");
  const [recipient, setRecipient] = useState("");
  const [recordsJson, setRecordsJson] = useState("[]");
  const [busy, setBusy] = useState<string | null>(null);

  const unlocked = signer?.unlocked ?? false;
  // Every action here is an on-chain spend that needs the name's owner coin from
  // a synced, address-indexed node. Gate all of them on write capability so the
  // user gets a clear reason instead of a "wallet does not hold …" failure.
  const canWrite = writeCap?.canWrite ?? false;
  const lock = !!busy || !canWrite;

  const run = async (
    label: string,
    builder: () => Promise<{ id: string }>,
  ) => {
    if (!profile) return;
    setBusy(label);
    try {
      const draft = await builder();
      const result = await exec.run(draft.id, profile.id, unlocked);
      showToast(`${label} broadcast — ${result.txid.slice(0, 12)}…`, "success");
      qc.invalidateQueries({ queryKey: ["wallet"] });
      onClose();
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setBusy(null);
    }
  };

  const parseRecords = (): Record<string, unknown>[] => {
    const v = JSON.parse(recordsJson);
    if (!Array.isArray(v)) throw new Error("records must be a JSON array");
    return v;
  };

  return (
    <Dialog open={open} onClose={onClose} title={`Manage .${name}`}>
      <div className="space-y-4 text-sm">
        {canWrite ? (
          <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
            Each action builds, signs (passphrase prompted in the secure window if
            locked), and broadcasts an on-chain covenant.
          </div>
        ) : (
          <div
            className="bg-red-50 border border-red-300 rounded p-2 text-xs text-red-800"
            role="alert"
            data-testid="name-actions-blocked"
          >
            <span className="font-semibold">Name actions unavailable.</span>{" "}
            {writeCap?.reason ??
              "Connect a fully-synced, address-indexed node and unlock your signer to manage names."}
          </div>
        )}

        {/* Auction lifecycle */}
        <section className="space-y-2">
          <div className="font-medium text-gray-700">Auction</div>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" disabled={lock} onClick={() => run("OPEN", () => build.open.mutateAsync({ name }))}>
              {busy === "OPEN" ? "…" : "Open"}
            </Button>
            <Button size="sm" disabled={lock} onClick={() => run("REVEAL", () => build.reveal.mutateAsync({ name }))}>
              {busy === "REVEAL" ? "…" : "Reveal"}
            </Button>
            <Button size="sm" disabled={lock} onClick={() => run("REDEEM", () => build.redeem.mutateAsync({ name }))}>
              {busy === "REDEEM" ? "…" : "Redeem"}
            </Button>
          </div>
          <div className="flex items-end gap-2">
            <Input label="Bid (HNS doos)" value={bidValue} onChange={(e) => setBidValue(e.target.value)} placeholder="1000000" />
            <Input label="Lockup (doos)" value={lockup} onChange={(e) => setLockup(e.target.value)} placeholder=">= bid" />
            <Button
              size="sm"
              disabled={lock || !bidValue || !lockup}
              onClick={() =>
                run("BID", () =>
                  build.bid.mutateAsync({
                    name,
                    bidValue: Number(bidValue),
                    lockup: Number(lockup),
                  }),
                )
              }
            >
              {busy === "BID" ? "…" : "Bid"}
            </Button>
          </div>
        </section>

        {/* Records (REGISTER / UPDATE) */}
        <section className="space-y-2">
          <div className="font-medium text-gray-700">Records (REGISTER / UPDATE)</div>
          <textarea
            className="w-full border border-gray-300 rounded px-2 py-1 font-mono text-xs h-20"
            value={recordsJson}
            onChange={(e) => setRecordsJson(e.target.value)}
            placeholder='[{"type":"TXT","txt":["hello"]}]'
          />
          <div className="flex gap-2">
            <Button
              size="sm"
              disabled={lock}
              onClick={() =>
                run("REGISTER", () =>
                  build.register.mutateAsync({ name, records: safeRecords(recordsJson) }),
                )
              }
            >
              {busy === "REGISTER" ? "…" : "Register"}
            </Button>
            <Button
              size="sm"
              disabled={lock}
              onClick={() => {
                let recs: Record<string, unknown>[];
                try {
                  recs = parseRecords();
                } catch (e) {
                  showToast(mapError(e), "error");
                  return;
                }
                run("UPDATE", () => build.update.mutateAsync({ name, records: recs }));
              }}
            >
              {busy === "UPDATE" ? "…" : "Update"}
            </Button>
          </div>
        </section>

        {/* Ownership / lifecycle */}
        <section className="space-y-2">
          <div className="font-medium text-gray-700">Ownership</div>
          <Input label="Transfer to address" value={recipient} onChange={(e) => setRecipient(e.target.value)} placeholder="hs1q… / rs1q…" />
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="danger"
              disabled={lock || !recipient.trim()}
              onClick={() => run("TRANSFER", () => build.transfer.mutateAsync({ name, recipient: recipient.trim() }))}
            >
              {busy === "TRANSFER" ? "…" : "Transfer"}
            </Button>
            <Button size="sm" disabled={lock} onClick={() => run("FINALIZE", () => build.finalize.mutateAsync({ name }))}>
              {busy === "FINALIZE" ? "…" : "Finalize"}
            </Button>
            <Button size="sm" disabled={lock} onClick={() => run("CANCEL", () => build.cancel.mutateAsync({ name }))}>
              {busy === "CANCEL" ? "…" : "Cancel transfer"}
            </Button>
            <Button size="sm" disabled={lock} onClick={() => run("RENEW", () => build.renew.mutateAsync({ name }))}>
              {busy === "RENEW" ? "…" : "Renew"}
            </Button>
            <Button size="sm" variant="danger" disabled={lock} onClick={() => run("REVOKE", () => build.revoke.mutateAsync({ name }))}>
              {busy === "REVOKE" ? "…" : "Revoke"}
            </Button>
          </div>
        </section>

        <div className="flex justify-end">
          <Button variant="ghost" onClick={onClose} disabled={!!busy}>Close</Button>
        </div>
      </div>
    </Dialog>
  );
}

/** Parse a records JSON string, returning `null` (→ EMPTY resource) on blank/invalid. */
function safeRecords(json: string): Record<string, unknown>[] | null {
  try {
    const v = JSON.parse(json);
    return Array.isArray(v) && v.length > 0 ? v : null;
  } catch {
    return null;
  }
}
