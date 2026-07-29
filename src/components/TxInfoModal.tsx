import { useReadTxInfo } from "../queries/read";
import { useNodeLive } from "../queries/node";
import { Dialog } from "./ui/Dialog";
import { Badge } from "./ui/Badge";
import { formatHns, formatDate, truncateMiddle } from "../lib/utils";
import { explorerTxUrl, openExternal } from "../lib/openExternal";
import { isTxInfoError } from "../types";

interface TxInfoModalProps {
  txid: string | null;
  open: boolean;
  onClose: () => void;
  /** Controls whether the "View on explorer" link renders (mainnet only). */
  isMainnet: boolean;
}

/**
 * Read-only modal displaying on-chain transaction details from the local hsd
 * node. Shows: status (Confirmed/Pending), confirmations, block height,
 * timestamp, fee, input/output counts, total output value.
 *
 * Gracefully degrades when the node is unavailable or the txid is unknown
 * (shows a "requires synced node" hint rather than erroring). Mirrors
 * `BlockInfoModal` — purely informational, no actions.
 */
export function TxInfoModal({ txid, open, onClose, isMainnet }: TxInfoModalProps) {
  const nodeLive = useNodeLive();
  const { data: result, isLoading } = useReadTxInfo(open ? txid : null);

  if (!open || !txid) return null;

  const indexDisabled = isTxInfoError(result);
  // Narrow to the tx object (excludes the error shape) for content rendering.
  const tx = indexDisabled ? null : result ?? null;
  const confirmed = tx != null && tx.confirmations > 0;

  return (
    <Dialog
      open={open}
      onClose={onClose}
      className="max-w-2xl"
      title={
        <>
          Transaction{" "}
          <span className="text-xs font-normal font-mono text-gray-400">
            {truncateMiddle(txid, 10, 6)}
          </span>
        </>
      }
    >
      <div className="space-y-4 text-sm max-h-[70vh] overflow-y-auto">
        {/* Explorer link (mainnet only) */}
        {isMainnet && (
          <button
            type="button"
            className="text-xs text-blue-500 hover:text-blue-700 hover:underline cursor-pointer inline-flex items-center gap-1"
            onClick={() => openExternal(explorerTxUrl(txid))}
            data-testid="tx-explorer-link"
          >
            View on explorer ↗
          </button>
        )}

        {/* Loading state */}
        {isLoading && (
          <div className="text-center py-4">
            <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-blue-600 mx-auto"></div>
            <div className="mt-2 text-sm text-gray-600">Loading transaction info...</div>
          </div>
        )}

        {/* Node lacks --index-tx — distinct signal, not a sync issue. */}
        {!isLoading && indexDisabled && (
          <div
            className="text-amber-700 bg-amber-50 border border-amber-200 rounded p-2 text-xs"
            data-testid="tx-info-index-disabled"
          >
            This node has transaction indexing disabled. Enable{" "}
            <code className="font-mono">--index-tx</code> on your hsd node to
            view transaction details.
          </div>
        )}

        {/* No node / degrade state */}
        {!isLoading && !indexDisabled && (!nodeLive || !tx) && (
          <div className="text-gray-500 text-xs" data-testid="tx-info-no-node">
            Requires a synced local node to display transaction details.
          </div>
        )}

        {/* Content */}
        {!isLoading && tx && (
          <div className="text-xs space-y-3">
            {/* Full txid */}
            <div className="space-y-1">
              <div className="text-gray-600">Transaction ID</div>
              <div
                className="font-mono break-all bg-gray-50 rounded p-1.5"
                data-testid="tx-hash"
              >
                {tx.txid}
              </div>
            </div>

            {/* Status */}
            <div className="flex justify-between items-center">
              <span className="text-gray-600">Status</span>
              <span data-testid="tx-status">
                {confirmed ? (
                  <Badge variant="success">Confirmed</Badge>
                ) : (
                  <Badge variant="warning">Pending</Badge>
                )}
              </span>
            </div>

            {/* Confirmations */}
            <div className="flex justify-between">
              <span className="text-gray-600">Confirmations</span>
              <span className="font-mono" data-testid="tx-confirmations">
                {tx.confirmations.toLocaleString()}
              </span>
            </div>

            {/* Block height */}
            <div className="flex justify-between">
              <span className="text-gray-600">Block height</span>
              <span className="font-mono" data-testid="tx-height">
                {tx.height >= 0 ? `#${tx.height.toLocaleString()}` : "—"}
              </span>
            </div>

            {/* Timestamp */}
            <div className="flex justify-between">
              <span className="text-gray-600">Timestamp</span>
              <span data-testid="tx-time">
                {tx.time > 0
                  ? formatDate(new Date(tx.time * 1000).toISOString())
                  : "—"}
              </span>
            </div>

            {/* Fee */}
            <div className="flex justify-between">
              <span className="text-gray-600">Fee</span>
              <span className="font-mono" data-testid="tx-fee">
                {tx.fee == null ? "—" : formatHns(tx.fee)}
              </span>
            </div>

            {/* Inputs */}
            <div className="flex justify-between">
              <span className="text-gray-600">Inputs</span>
              <span className="font-mono" data-testid="tx-inputs">
                {tx.inputsCount.toLocaleString()}
              </span>
            </div>

            {/* Outputs */}
            <div className="flex justify-between">
              <span className="text-gray-600">Outputs</span>
              <span className="font-mono" data-testid="tx-outputs">
                {tx.outputsCount.toLocaleString()}
              </span>
            </div>

            {/* Total out */}
            <div className="flex justify-between">
              <span className="text-gray-600">Total out</span>
              <span className="font-mono" data-testid="tx-total-out">
                {formatHns(tx.totalOut)}
              </span>
            </div>
          </div>
        )}
      </div>
    </Dialog>
  );
}
