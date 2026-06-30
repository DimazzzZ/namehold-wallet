import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  useActiveProfile,
  useSignerSession,
  useWriteCapability,
  useNameAction,
  useExecuteDraft,
} from "../queries/wallet";
import { useReadNameInfo } from "../queries/read";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { Dialog } from "./ui/Dialog";
import { Badge } from "./ui/Badge";
import { useUiStore } from "../stores/ui";
import { mapError } from "../lib/errors";
import { formatHns } from "../lib/utils";
import {
  auctionPhase,
  nextTransition,
  formatCountdown,
  recommendedAction,
} from "../lib/auction";
import {
  DNS_RECORD_TYPES,
  rowsToRecords,
  valuePlaceholder,
  type DnsRecordType,
  type DnsRow,
} from "../lib/dnsRecords";

/**
 * One modal that exposes every name covenant action for a single name, wired to
 * the `build_*_draft` commands + the build→unlock→sign→broadcast runner.
 *
 * The header shows the name's live auction phase (from `read_name_info`) with a
 * countdown to the next transition and highlights the recommended action.
 * Records (REGISTER/UPDATE) use a typed row editor; an Advanced toggle exposes
 * the raw-JSON array for record types the row editor doesn't cover (DS, GLUE…).
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
  const { data: info } = useReadNameInfo(open ? name : null);
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
  const [rows, setRows] = useState<DnsRow[]>([{ type: "TXT", value: "" }]);
  const [advanced, setAdvanced] = useState(false);
  const [recordsJson, setRecordsJson] = useState("[]");
  const [busy, setBusy] = useState<string | null>(null);

  const unlocked = signer?.unlocked ?? false;
  // Every action here is an on-chain spend that needs the name's owner coin from
  // a synced, address-indexed node. Gate all of them on write capability so the
  // user gets a clear reason instead of a "wallet does not hold …" failure.
  const canWrite = writeCap?.canWrite ?? false;
  const lock = !!busy || !canWrite;

  const badge = auctionPhase(info?.state);
  const countdown = nextTransition(info?.state, info?.stats);
  const recommended = recommendedAction(info?.state);

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

  // Records for submit: typed rows by default, raw-JSON array in Advanced mode.
  const recordsForSubmit = (): Record<string, unknown>[] | null => {
    if (advanced) {
      const v = JSON.parse(recordsJson);
      if (!Array.isArray(v)) throw new Error("records must be a JSON array");
      return v.length > 0 ? v : null;
    }
    return rowsToRecords(rows);
  };

  const submitRecords = (label: "REGISTER" | "UPDATE") => {
    let recs: Record<string, unknown>[] | null;
    try {
      recs = recordsForSubmit();
    } catch (e) {
      showToast(mapError(e), "error");
      return;
    }
    const builder = label === "REGISTER" ? build.register : build.update;
    run(label, () => builder.mutateAsync({ name, records: recs }));
  };

  const setRow = (i: number, patch: Partial<DnsRow>) =>
    setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const addRow = () => setRows((rs) => [...rs, { type: "TXT", value: "" }]);
  const removeRow = (i: number) =>
    setRows((rs) => (rs.length > 1 ? rs.filter((_, j) => j !== i) : rs));

  // Highlight the phase-recommended action button.
  const isRec = (key: string) => recommended?.key === key;

  return (
    <Dialog open={open} onClose={onClose} title={`Manage .${name}`}>
      <div className="space-y-4 text-sm">
        {/* Phase header */}
        <div
          className="flex items-center justify-between gap-3 bg-gray-50 border border-gray-200 rounded p-2"
          data-testid="name-phase"
        >
          <div className="flex items-center gap-2">
            <Badge variant={badge.variant}>{badge.label}</Badge>
            {countdown && (
              <span className="text-xs text-gray-600" data-testid="name-countdown">
                {countdown.label} {formatCountdown(countdown)}
              </span>
            )}
          </div>
          {(info?.highest ?? info?.value) != null && (
            <span className="text-xs text-gray-500">
              {info?.highest != null ? `High bid ${formatHns(info.highest)} HNS` : ""}
              {info?.value != null ? ` · value ${formatHns(info.value)} HNS` : ""}
            </span>
          )}
        </div>

        {recommended && canWrite && (
          <div
            className="bg-blue-50 border border-blue-200 rounded p-2 text-xs text-blue-800"
            data-testid="name-recommended"
          >
            <strong>{recommended.label}:</strong> {recommended.hint}
          </div>
        )}

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
            <Button size="sm" variant={isRec("OPEN") ? "primary" : "secondary"} disabled={lock} onClick={() => run("OPEN", () => build.open.mutateAsync({ name }))}>
              {busy === "OPEN" ? "…" : "Open"}
            </Button>
            <Button size="sm" variant={isRec("REVEAL") ? "primary" : "secondary"} disabled={lock} onClick={() => run("REVEAL", () => build.reveal.mutateAsync({ name }))}>
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
              variant={isRec("BID") ? "primary" : "secondary"}
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
          <div className="flex items-center justify-between">
            <div className="font-medium text-gray-700">DNS records (REGISTER / UPDATE)</div>
            <button
              type="button"
              className="text-xs text-blue-600 hover:underline"
              onClick={() => setAdvanced((a) => !a)}
              data-testid="dns-advanced-toggle"
            >
              {advanced ? "Use row editor" : "Advanced (raw JSON)"}
            </button>
          </div>

          {advanced ? (
            <textarea
              className="w-full border border-gray-300 rounded px-2 py-1 font-mono text-xs h-20"
              value={recordsJson}
              onChange={(e) => setRecordsJson(e.target.value)}
              placeholder='[{"type":"TXT","txt":["hello"]}]'
              data-testid="dns-json"
            />
          ) : (
            <div className="space-y-2" data-testid="dns-rows">
              {rows.map((row, i) => (
                <div key={i} className="flex items-center gap-2">
                  <select
                    className="border border-gray-300 rounded px-2 py-1 text-xs"
                    value={row.type}
                    onChange={(e) => setRow(i, { type: e.target.value as DnsRecordType })}
                    aria-label="record type"
                  >
                    {DNS_RECORD_TYPES.map((t) => (
                      <option key={t} value={t}>
                        {t}
                      </option>
                    ))}
                  </select>
                  <input
                    className="flex-1 border border-gray-300 rounded px-2 py-1 text-xs font-mono"
                    value={row.value}
                    onChange={(e) => setRow(i, { value: e.target.value })}
                    placeholder={valuePlaceholder(row.type)}
                    aria-label="record value"
                  />
                  <button
                    type="button"
                    className="text-xs text-gray-400 hover:text-red-600 px-1"
                    onClick={() => removeRow(i)}
                    aria-label="remove record"
                  >
                    ✕
                  </button>
                </div>
              ))}
              <button
                type="button"
                className="text-xs text-blue-600 hover:underline"
                onClick={addRow}
                data-testid="dns-add-row"
              >
                + Add record
              </button>
            </div>
          )}

          <div className="flex gap-2">
            <Button size="sm" variant={isRec("REGISTER") ? "primary" : "secondary"} disabled={lock} onClick={() => submitRecords("REGISTER")}>
              {busy === "REGISTER" ? "…" : "Register"}
            </Button>
            <Button size="sm" disabled={lock} onClick={() => submitRecords("UPDATE")}>
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
