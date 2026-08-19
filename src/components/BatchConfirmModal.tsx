import { useState } from "react";
import { Dialog } from "./ui/Dialog";
import { Button } from "./ui/Button";
import { formatHns } from "../lib/utils";
import { displayName } from "../lib/idn";

export interface BatchConfirmModalProps {
  open: boolean;
  action: "bid" | "renew" | "reveal" | "redeem" | "finalize";
  names: string[];
  estimatedFeeDoos: number;
  onConfirm: () => void | Promise<void>;
  onCancel: () => void;
}

/**
 * Batch confirmation modal: shows the user what they're about to do (action +
 * name count + estimated fee) and asks for confirmation before proceeding.
 * Names list is collapsible (collapsed by default for large batches).
 */
export function BatchConfirmModal({
  open,
  action,
  names,
  estimatedFeeDoos,
  onConfirm,
  onCancel,
}: BatchConfirmModalProps) {
  const [expanded, setExpanded] = useState(false);
  const [confirming, setConfirming] = useState(false);

  const actionLabel = {
    bid: "bid",
    renew: "renew",
    reveal: "reveal",
    redeem: "redeem",
    finalize: "finalize",
  }[action];

  const handleConfirm = async () => {
    setConfirming(true);
    try {
      await onConfirm();
    } finally {
      setConfirming(false);
    }
  };

  return (
    <Dialog open={open} onClose={onCancel} title={`Confirm batch ${actionLabel}`}>
      <div className="space-y-4">
        <div className="bg-blue-50 border border-blue-200 rounded p-3 text-sm space-y-2">
          <div>
            <span className="text-gray-700">
              You&apos;re about to <strong>{actionLabel}</strong>{" "}
              <strong>{names.length}</strong> name
              {names.length !== 1 ? "s" : ""}.
            </span>
          </div>
          <div>
            <span className="text-gray-700">
              Estimated fee: <strong>~{formatHns(estimatedFeeDoos / 1_000_000)}</strong> HNS
            </span>
          </div>
        </div>

        {/* Collapsible name list */}
        <div className="border border-gray-200 rounded">
          <button
            type="button"
            className="w-full text-left px-3 py-2 hover:bg-gray-50 flex items-center justify-between text-sm"
            onClick={() => setExpanded((prev) => !prev)}
          >
            <span className="font-medium text-gray-700">
              {expanded ? "\u25BC" : "\u25B6"} Names ({names.length})
            </span>
          </button>
          {expanded && (
            <div className="border-t border-gray-200 bg-gray-50 max-h-48 overflow-auto">
              <ul className="divide-y divide-gray-200">
                {names.map((name) => (
                  <li key={name} className="px-3 py-2 text-xs font-mono text-gray-700">
                    .{displayName(name)}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>

        {/* Action buttons */}
        <div className="flex gap-2 justify-end pt-2">
          <Button variant="ghost" onClick={onCancel} disabled={confirming}>
            Cancel
          </Button>
          <Button variant="primary" onClick={handleConfirm} disabled={confirming}>
            {confirming ? "Processing\u2026" : "Confirm"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
