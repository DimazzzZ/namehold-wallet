import { useMemo } from "react";
import {
  useNamebaseDomainWithdrawals,
  useNamebaseWithdrawals,
  namebaseStatus,
} from "../queries/namebase";
import { useActiveProfile } from "../queries/wallet";
import { Badge } from "./ui/Badge";
import { EmptyState } from "./ui/EmptyState";
import { formatDate, formatHns, truncate } from "../lib/utils";

/**
 * Transfers the user has moved off Namebase, shown exactly as Namebase reports
 * them so the app mirrors Namebase's own lists + statuses:
 *   - Domain transfers from `/api/domains/withdrawals` (transfer_/finalize_ statuses).
 *   - HNS withdrawals from `/api/withdrawals` (pending/completed).
 */
export function TransfersView() {
  const { data: domainTransfers = [], isLoading } = useNamebaseDomainWithdrawals();
  const { data: profile } = useActiveProfile();

  const { data: withdrawals = [] } = useNamebaseWithdrawals();
  const hnsWithdrawals = useMemo(
    () => withdrawals.filter((w) => w.currency?.toLowerCase() === "hns"),
    [withdrawals],
  );

  const myAddress = profile?.receiveAddress ?? null;

  return (
    <div className="space-y-6">
    <div className="bg-white rounded-lg border border-gray-200 p-4">
      <h3 className="text-sm font-semibold text-gray-700 mb-1">Domain transfers</h3>
      <p className="text-xs text-gray-500 mb-3">
        Transfers off Namebase, with Namebase's live status for each domain.
      </p>
      {isLoading ? (
        <div className="text-sm text-gray-400 py-4 text-center">Loading…</div>
      ) : domainTransfers.length === 0 ? (
        <EmptyState
          title="No transfers yet"
          description="Transfer a domain from the Namebase tab and it'll appear here with live status."
        />
      ) : (
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-gray-500 border-b border-gray-100">
              <th className="py-1 font-medium">Domain</th>
              <th className="py-1 font-medium">Destination</th>
              <th className="py-1 font-medium">Status</th>
              <th className="py-1 font-medium">Updated</th>
            </tr>
          </thead>
          <tbody>
            {domainTransfers.map((t) => {
              const { label, tone } = namebaseStatus(t.status);
              return (
                <tr key={t.id ?? t.domain} className="border-b border-gray-50">
                  <td className="py-2 font-mono">.{t.domain}</td>
                  <td className="py-2 font-mono text-xs text-gray-500">
                    {truncate(t.destination_address, 16)}
                    {!!myAddress && t.destination_address === myAddress && (
                      <span className="ml-1 text-gray-400">(your wallet)</span>
                    )}
                  </td>
                  <td className="py-2">
                    <Badge variant={tone} title={t.status_note ?? undefined}>
                      {label}
                    </Badge>
                  </td>
                  <td className="py-2 text-xs text-gray-400">
                    {t.updated_at || t.created_at
                      ? formatDate(t.updated_at || t.created_at)
                      : "—"}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>

    <div className="bg-white rounded-lg border border-gray-200 p-4">
      <h3 className="text-sm font-semibold text-gray-700 mb-1">HNS withdrawals</h3>
      <p className="text-xs text-gray-500 mb-3">
        HNS funds withdrawn from your Namebase balance, with Namebase's status.
      </p>
      {hnsWithdrawals.length === 0 ? (
        <EmptyState
          title="No withdrawals yet"
          description="Use 'Withdraw HNS' on the Namebase tab to move funds to an address."
        />
      ) : (
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-gray-500 border-b border-gray-100">
              <th className="py-1 font-medium">Amount</th>
              <th className="py-1 font-medium">Destination</th>
              <th className="py-1 font-medium">Status</th>
              <th className="py-1 font-medium">Date</th>
            </tr>
          </thead>
          <tbody>
            {hnsWithdrawals.map((w) => {
              const { label, tone } = namebaseStatus(w.status);
              return (
                <tr key={w.id} className="border-b border-gray-50">
                  <td className="py-2 font-mono">{formatHns(Number(w.amount) || 0)} HNS</td>
                  <td className="py-2 font-mono text-xs text-gray-500">
                    {truncate(w.destination_address, 16)}
                    {!!myAddress && w.destination_address === myAddress && (
                      <span className="ml-1 text-gray-400">(your wallet)</span>
                    )}
                  </td>
                  <td className="py-2">
                    <Badge variant={tone} title={w.status_note ?? undefined}>
                      {label}
                    </Badge>
                  </td>
                  <td className="py-2 text-xs text-gray-400">
                    {w.created_at ? formatDate(w.created_at) : "—"}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
    </div>
  );
}
