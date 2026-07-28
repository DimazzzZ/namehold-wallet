import { useReadBlockInfo } from "../queries/read";
import { useNodeLive } from "../queries/node";
import { Dialog } from "./ui/Dialog";
import { formatHns, formatDateLong } from "../lib/utils";
import { explorerBlockUrl, openExternal } from "../lib/openExternal";

interface BlockInfoModalProps {
  height: number | null;
  open: boolean;
  onClose: () => void;
  /** Controls whether the "View on explorer" link renders (mainnet only). */
  isMainnet: boolean;
}

/**
 * Read-only modal displaying on-chain block details from the local hsd node.
 * Shows: height, hash, timestamp, tx count, miner reward, difficulty.
 * Gracefully degrades when the node is unavailable (shows a "requires synced
 * node" hint rather than erroring).
 *
 * Mirrors `NameInfoModal` — purely informational, no actions.
 */
export function BlockInfoModal({ height, open, onClose, isMainnet }: BlockInfoModalProps) {
  const nodeLive = useNodeLive();
  const { data: block, isLoading } = useReadBlockInfo(open ? height : null);

  if (!open || height == null) return null;

  return (
    <Dialog
      open={open}
      onClose={onClose}
      className="max-w-2xl"
      title={`Block #${height.toLocaleString()}`}
    >
      <div className="space-y-4 text-sm max-h-[70vh] overflow-y-auto">
        {/* Explorer link (mainnet only) */}
        {isMainnet && (
          <button
            type="button"
            className="text-xs text-blue-500 hover:text-blue-700 hover:underline cursor-pointer inline-flex items-center gap-1"
            onClick={() => openExternal(explorerBlockUrl(height))}
            data-testid="block-explorer-link"
          >
            View on explorer ↗
          </button>
        )}

        {/* Loading state */}
        {isLoading && (
          <div className="text-center py-4">
            <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-blue-600 mx-auto"></div>
            <div className="mt-2 text-sm text-gray-600">Loading block info...</div>
          </div>
        )}

        {/* No node / degrade state */}
        {!isLoading && (!nodeLive || !block) && (
          <div className="text-gray-500 text-xs" data-testid="block-info-no-node">
            Requires a synced local node to display block details.
          </div>
        )}

        {/* Content */}
        {!isLoading && block && (
          <div className="text-xs space-y-3">
            {/* Hash */}
            <div className="space-y-1">
              <div className="text-gray-600">Hash</div>
              <div className="font-mono break-all bg-gray-50 rounded p-1.5" data-testid="block-hash">
                {block.hash}
              </div>
            </div>

            {/* Timestamp */}
            <div className="flex justify-between">
              <span className="text-gray-600">Timestamp</span>
              <span data-testid="block-time">
                {block.time > 0
                  ? formatDateLong(new Date(block.time * 1000).toISOString())
                  : "—"}
              </span>
            </div>

            {/* Tx count */}
            <div className="flex justify-between">
              <span className="text-gray-600">Transactions</span>
              <span className="font-mono" data-testid="block-tx-count">
                {block.txCount.toLocaleString()}
              </span>
            </div>

            {/* Miner reward */}
            <div className="flex justify-between">
              <span className="text-gray-600">Miner reward</span>
              <span className="font-mono" data-testid="block-miner-reward">
                {formatHns(block.minerReward)}
              </span>
            </div>

            {/* Difficulty */}
            <div className="flex justify-between">
              <span className="text-gray-600">Difficulty</span>
              <span className="font-mono" data-testid="block-difficulty">
                {block.difficulty.toLocaleString(undefined, {
                  maximumFractionDigits: 4,
                })}
              </span>
            </div>
          </div>
        )}
      </div>
    </Dialog>
  );
}
