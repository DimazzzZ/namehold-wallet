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
  AUCTION_PHASE_GUIDE,
  hnsToDoos,
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
 * The modal is phase-guided: it shows the most relevant action for the name's
 * current auction phase prominently, with a clear description. Advanced actions
 * (transfer, revoke, etc.) are behind an "All actions" toggle.
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
  const { data: info, isLoading, isError, error } = useReadNameInfo(open ? name : null);
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

  // Bid inputs in HNS (human-readable), converted to doos on submit.
  const [bidHns, setBidHns] = useState("");
  const [lockupHns, setLockupHns] = useState("");
  const [recipient, setRecipient] = useState("");
  const [rows, setRows] = useState<DnsRow[]>([{ type: "TXT", value: "" }]);
  const [advanced, setAdvanced] = useState(false);
  const [recordsJson, setRecordsJson] = useState("[]");
  const [showAllActions, setShowAllActions] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);

  const unlocked = signer?.unlocked ?? false;
  const canWrite = writeCap?.canWrite ?? false;
  const lock = !!busy || !canWrite;

  const badge = auctionPhase(info?.state);
  const countdown = nextTransition(info?.state, info?.stats);
  const guide = AUCTION_PHASE_GUIDE[badge.phase];

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

  // Render the phase-specific guided action.
  const renderGuidedAction = () => {
    if (!guide) return null;

    switch (badge.phase) {
      case "AVAILABLE":
        return (
          <div className="space-y-2">
            <div className="text-sm text-gray-700">{guide.description}</div>
            <Button
              variant="primary"
              disabled={lock}
              onClick={() => run("OPEN", () => build.open.mutateAsync({ name }))}
            >
              {busy === "OPEN" ? "Opening…" : guide.action}
            </Button>
          </div>
        );

      case "OPENING":
        return (
          <div className="text-sm text-gray-600">
            {guide.description}
            {countdown && (
              <div className="mt-1 font-medium">
                {countdown.label} {formatCountdown(countdown)}
              </div>
            )}
          </div>
        );

      case "BIDDING":
        return (
          <div className="space-y-3">
            <div className="text-sm text-gray-700">{guide.description}</div>
            <div className="flex items-end gap-2">
              <div className="flex-1">
                <Input
                  label="Bid (HNS)"
                  value={bidHns}
                  onChange={(e) => setBidHns(e.target.value)}
                  placeholder="10.0"
                  type="number"
                  step="0.000001"
                />
              </div>
              <div className="flex-1">
                <Input
                  label="Lockup (HNS)"
                  value={lockupHns}
                  onChange={(e) => setLockupHns(e.target.value)}
                  placeholder="≥ bid"
                  type="number"
                  step="0.000001"
                />
              </div>
            </div>
            <div className="text-xs text-gray-500">
              Lockup must be ≥ bid. Excess is returned after reveal.
            </div>
            <Button
              variant="primary"
              disabled={lock || !bidHns || !lockupHns}
              onClick={() =>
                run("BID", () =>
                  build.bid.mutateAsync({
                    name,
                    bidValue: hnsToDoos(Number(bidHns)),
                    lockup: hnsToDoos(Number(lockupHns)),
                  }),
                )
              }
            >
              {busy === "BID" ? "Placing bid…" : guide.action}
            </Button>
          </div>
        );

      case "REVEAL":
        return (
          <div className="space-y-2">
            <div className="text-sm text-gray-700">{guide.description}</div>
            <div className="flex gap-2">
              <Button
                variant="primary"
                disabled={lock}
                onClick={() => run("REVEAL", () => build.reveal.mutateAsync({ name }))}
              >
                {busy === "REVEAL" ? "Revealing…" : guide.action}
              </Button>
              <Button
                variant="secondary"
                disabled={lock}
                onClick={() => run("REDEEM", () => build.redeem.mutateAsync({ name }))}
              >
                {busy === "REDEEM" ? "…" : "Redeem (lost bid)"}
              </Button>
            </div>
          </div>
        );

      case "CLOSED":
        return (
          <div className="space-y-3">
            <div className="text-sm text-gray-700">{guide.description}</div>
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
            <Button
              variant="primary"
              disabled={lock}
              onClick={() => submitRecords("REGISTER")}
            >
              {busy === "REGISTER" ? "Registering…" : guide.action}
            </Button>
          </div>
        );

      default:
        return null;
    }
  };

  // Determine if the name is owned by the current wallet (has an owner and is registered)
  const isOwned = !!info?.owner && info?.registered === true;

  return (
    <Dialog open={open} onClose={onClose} title={`.${name}`}>
      <div className="space-y-4 text-sm">
        {/* Loading state */}
        {isLoading && (
          <div className="text-center py-4">
            <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-blue-600 mx-auto"></div>
            <div className="mt-2 text-sm text-gray-600">Loading name info...</div>
          </div>
        )}

        {/* Error state */}
        {isError && (
          <div className="bg-red-50 border border-red-300 rounded p-3 text-sm text-red-800">
            <div className="font-medium">Failed to load name info</div>
            <div className="mt-1 text-xs">{error?.message || "Unknown error"}</div>
          </div>
        )}

        {/* Phase header - only show when data is loaded and no error */}
        {!isLoading && !isError && (
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
        )}

        {/* Write-capability gate */}
        {!canWrite && (
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

        {/* Phase-guided primary action - only show when data is loaded and no error */}
        {!isLoading && !isError && guide && (
          <div className="bg-blue-50 border border-blue-200 rounded p-3">
            <div className="font-medium text-blue-900 mb-2">{guide.title}</div>
            {renderGuidedAction()}
          </div>
        )}

        {/* Advanced actions toggle */}
        <div>
          <button
            type="button"
            className="text-xs text-gray-500 hover:text-gray-700 hover:underline"
            onClick={() => setShowAllActions((a) => !a)}
            data-testid="all-actions-toggle"
          >
            {showAllActions ? "Hide advanced actions" : "Show all actions"}
          </button>
        </div>

        {showAllActions && (
          <div className="space-y-4 border-t border-gray-200 pt-4">
            {/* Auction actions - always show for all names */}
            <section className="space-y-2">
              <div className="font-medium text-gray-700">Auction</div>
              <div className="flex flex-wrap gap-2">
                <Button size="sm" variant="secondary" disabled={lock} onClick={() => run("OPEN", () => build.open.mutateAsync({ name }))}>
                  {busy === "OPEN" ? "…" : "Open"}
                </Button>
                <Button size="sm" variant="secondary" disabled={lock} onClick={() => run("REVEAL", () => build.reveal.mutateAsync({ name }))}>
                  {busy === "REVEAL" ? "…" : "Reveal"}
                </Button>
                <Button size="sm" variant="secondary" disabled={lock} onClick={() => run("REDEEM", () => build.redeem.mutateAsync({ name }))}>
                  {busy === "REDEEM" ? "…" : "Redeem"}
                </Button>
              </div>
              <div className="flex items-end gap-2">
                <Input label="Bid (HNS)" value={bidHns} onChange={(e) => setBidHns(e.target.value)} placeholder="10.0" type="number" step="0.000001" />
                <Input label="Lockup (HNS)" value={lockupHns} onChange={(e) => setLockupHns(e.target.value)} placeholder="≥ bid" type="number" step="0.000001" />
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={lock || !bidHns || !lockupHns}
                  onClick={() =>
                    run("BID", () =>
                      build.bid.mutateAsync({
                        name,
                        bidValue: hnsToDoos(Number(bidHns)),
                        lockup: hnsToDoos(Number(lockupHns)),
                      }),
                    )
                  }
                >
                  {busy === "BID" ? "…" : "Bid"}
                </Button>
              </div>
            </section>

            {/* DNS records (REGISTER / UPDATE) - only show for owned names or when in CLOSED phase */}
            {(isOwned || badge.phase === "CLOSED") && (
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
                  <div className="space-y-2" data-testid="dns-rows-advanced">
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
                      data-testid="dns-add-row-advanced"
                    >
                      + Add record
                    </button>
                  </div>
                )}

                <div className="flex gap-2">
                  <Button size="sm" variant="secondary" disabled={lock} onClick={() => submitRecords("REGISTER")}>
                    {busy === "REGISTER" ? "…" : "Register"}
                  </Button>
                  <Button size="sm" disabled={lock} onClick={() => submitRecords("UPDATE")}>
                    {busy === "UPDATE" ? "…" : "Update"}
                  </Button>
                </div>
              </section>
            )}

            {/* Ownership / lifecycle - only show for owned names */}
            {isOwned && (
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
            )}
          </div>
        )}

        <div className="flex justify-end">
          <Button variant="ghost" onClick={onClose} disabled={!!busy}>Close</Button>
        </div>
      </div>
    </Dialog>
  );
}
