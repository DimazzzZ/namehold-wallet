import { Button } from "../ui/Button";
import { ActionHint } from "../name-actions/ActionHint";
import { DnsRecordsEditor } from "../name-actions/DnsRecordsEditor";
import { ReadOnlyDnsRecords } from "../name-actions/ReadOnlyDnsRecords";
import { UpcomingSection } from "../name-actions/UpcomingSection";
import { TAB_GRID } from "./OverviewTab";
import { RECORDS_NOT_FRESH_REASON, type NameActions } from "./useNameActions";

/** DNS records: the editor for a registered name of ours, read-only otherwise. */
export function DnsTab({ a }: { a: NameActions }) {
  const { name, caps, sections, busy, recordsFresh, recordsFetching, currentRecords } = a;

  const body = (() => {
    if (sections.records.kind === "upcoming") {
      return <UpcomingSection id="records" title="DNS records" when={sections.records.when} />;
    }
    if (sections.records.kind !== "live") {
      return <ReadOnlyDnsRecords name={name} profileId={a.profile?.id ?? null} />;
    }
    return (
      <section className="space-y-2">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-gray-900">DNS records</h3>
          <button
            type="button"
            className="text-xs text-blue-600 hover:underline"
            onClick={() => a.setAdvanced((v) => !v)}
            data-testid="dns-advanced-toggle"
          >
            {a.advanced ? "Use row editor" : "Advanced (raw JSON)"}
          </button>
        </div>

        {/* Freshness gate: the editor seeds and UPDATE is enabled ONLY from a
            guaranteed-fresh read of the current on-chain records. */}
        {recordsFetching && (
          <div className="text-xs text-gray-500" data-testid="dns-records-loading">
            Loading current on-chain records…
          </div>
        )}
        {!recordsFetching && !recordsFresh && (
          <div
            className="text-xs text-red-700 bg-red-50 border border-red-200 rounded p-2 flex items-center justify-between gap-2"
            data-testid="dns-records-stale-banner"
          >
            <span>
              Can&apos;t read this name&apos;s current on-chain records. The Update button is
              disabled to avoid overwriting your records from an incomplete view — make sure your
              node is running and fully synced, then retry.
            </span>
            <button
              type="button"
              className="shrink-0 text-blue-600 hover:underline"
              onClick={() => a.refetchRecords()}
              data-testid="dns-records-retry"
            >
              Retry
            </button>
          </div>
        )}
        {recordsFresh && currentRecords?.records?.length === 0 && (
          <div className="text-xs text-gray-400" data-testid="dns-records-hint">
            This name has no records yet. Add records below and Update to publish them.
          </div>
        )}

        {/* The editor renders only once the fresh read has seeded it, so the
            seed never clobbers typing. */}
        {!recordsFresh ? null : a.advanced ? (
          <textarea
            className="w-full border border-gray-300 rounded px-2 py-1 font-mono text-xs h-20"
            value={a.recordsJson}
            onChange={(e) => a.setRecordsJson(e.target.value)}
            placeholder='[{"type":"TXT","txt":["hello"]}]'
            data-testid="dns-json"
          />
        ) : (
          <DnsRecordsEditor
            variant="advanced"
            rows={a.rows}
            onRowChange={a.setRow}
            onAddRow={a.addRow}
            onRemoveRow={a.removeRow}
          />
        )}

        <div className="flex gap-2">
          {/* The guided panel owns Register whenever it is the step the name
              is on. */}
          {caps?.taskState !== "wonNeedsRegister" && (
            <ActionHint
              reason={!recordsFresh ? RECORDS_NOT_FRESH_REASON : a.actionReason(caps?.canRegister)}
            >
              <Button
                size="sm"
                variant="secondary"
                disabled={a.actionDisabled("REGISTER", caps?.canRegister) || !recordsFresh}
                onClick={() => a.submitRecords("REGISTER")}
              >
                {busy === "REGISTER" ? "…" : "Register"}
              </Button>
            </ActionHint>
          )}
          <ActionHint
            reason={!recordsFresh ? RECORDS_NOT_FRESH_REASON : a.actionReason(caps?.canUpdate)}
          >
            <Button
              size="sm"
              variant="secondary"
              disabled={a.actionDisabled("UPDATE", caps?.canUpdate) || !recordsFresh}
              onClick={() => a.submitRecords("UPDATE")}
            >
              {busy === "UPDATE" ? "…" : "Publish changes (Update)"}
            </Button>
          </ActionHint>
        </div>
      </section>
    );
  })();

  return (
    <div className={TAB_GRID}>
      <div className="bg-white border border-gray-200 rounded-lg p-4 flex flex-col gap-3 min-w-0">
        {body}
      </div>
      <aside className="bg-white border border-gray-200 rounded-lg p-4 flex flex-col gap-2 text-[13px] text-gray-700">
        <h3 className="text-sm font-semibold text-gray-900">What records do</h3>
        <p>
          NS points the name at the servers that answer for it; DS pins its DNSSEC key; GLUE and
          SYNTH carry addresses; TXT holds free text.
        </p>
        <p>Update replaces every record at once — rows you remove here disappear on-chain.</p>
      </aside>
    </div>
  );
}
