import { ArrowRight } from "lucide-react";
import { Icon } from "../ui/Icon";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { isToFinish, isWaiting } from "../../lib/activity";
import { explorerCoversNetwork } from "../../lib/openExternal";
import { fromInteractiveChild } from "../../lib/rowClick";
import { useMergedActivity } from "../../queries/activity";
import { useActiveProfile } from "../../queries/wallet";
import { TxInfoModal } from "../TxInfoModal";
import { Tooltip } from "../ui/Tooltip";
import { AmountCell, DateCell, StatusCell, WhatCell } from "./cells";

/**
 * The Wallet's summary of the 5 newest rows. No actions here: a row that
 * still needs you opens Activity, a finished one its transaction.
 */
export function RecentActivityCard({ onNameClick }: { onNameClick: (name: string) => void }) {
  const navigate = useNavigate();
  const { rows } = useMergedActivity();
  const { data: profile } = useActiveProfile();
  const [infoTx, setInfoTx] = useState<string | null>(null);
  const recent = rows.slice(0, 5);

  return (
    <section data-testid="recent-activity" className="rounded-md border border-gray-200 bg-white">
      <div className="px-4 py-3 border-b border-gray-100 flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold text-gray-700">Recent activity</h3>
        <button
          type="button"
          className="text-xs text-blue-600 hover:underline inline-flex items-center gap-1"
          onClick={() => navigate("/activity")}
        >
          All activity <Icon icon={ArrowRight} size={12} />
        </button>
      </div>
      {recent.length === 0 ? (
        <div className="py-4 text-center text-sm text-gray-400">No activity yet.</div>
      ) : (
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-gray-500 border-b">
              <th className="py-1 pr-4 pl-4 w-[170px]">Date</th>
              <th className="py-1 pr-4">What</th>
              <th className="py-1 pr-4 text-right">Amount (HNS)</th>
              <th className="py-1">Status</th>
            </tr>
          </thead>
          <tbody>
            {recent.map((row) => {
              const toActivity = isToFinish(row) || isWaiting(row) || !row.txid;
              return (
                <tr
                  key={row.key}
                  className="border-t border-gray-100 hover:bg-gray-50 cursor-pointer"
                  data-testid="recent-activity-row"
                  onClick={(e) => {
                    if (fromInteractiveChild(e)) return;
                    if (toActivity) navigate("/activity");
                    else setInfoTx(row.txid);
                  }}
                >
                  <td className="py-1 pr-4 pl-4 text-xs text-gray-500 whitespace-nowrap">
                    {/* A Tooltip can't wrap a <tr>: the row's hint sits on its date. */}
                    <Tooltip content={toActivity ? "Open in Activity" : "View transaction info"}>
                      <span>
                        <DateCell row={row} />
                      </span>
                    </Tooltip>
                  </td>
                  <td className="py-1 pr-4">
                    <WhatCell row={row} onNameClick={onNameClick} />
                  </td>
                  <td className="py-1 pr-4 text-right text-xs whitespace-nowrap">
                    <AmountCell row={row} />
                  </td>
                  <td className="py-1">
                    <StatusCell row={row} />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      {infoTx != null && (
        <TxInfoModal
          txid={infoTx}
          open
          onClose={() => setInfoTx(null)}
          isMainnet={explorerCoversNetwork(profile?.network)}
        />
      )}
    </section>
  );
}
