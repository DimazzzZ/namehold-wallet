import type { MigrationStatus } from "../../types";

const STATUS_COLORS: Record<MigrationStatus, string> = {
  not_started: "bg-gray-200 text-gray-700",
  namebase_transfer_requested: "bg-yellow-100 text-yellow-800",
  waiting_transfer_tx: "bg-orange-100 text-orange-800",
  transfer_seen_on_chain: "bg-blue-100 text-blue-800",
  waiting_finalize: "bg-indigo-100 text-indigo-800",
  finalized_owned: "bg-green-100 text-green-800",
  failed_or_stuck: "bg-red-100 text-red-800",
  do_not_touch_staked: "bg-purple-100 text-purple-800",
};

const STATUS_LABELS: Record<MigrationStatus, string> = {
  not_started: "Not Started",
  namebase_transfer_requested: "Transfer Requested",
  waiting_transfer_tx: "Waiting TX",
  transfer_seen_on_chain: "TX Seen",
  waiting_finalize: "Waiting Finalize",
  finalized_owned: "Finalized",
  failed_or_stuck: "Failed/Stuck",
  do_not_touch_staked: "Do Not Touch",
};

export function StatusBadge({ status }: { status: MigrationStatus }) {
  return (
    <span
      className={`inline-block px-2 py-0.5 rounded text-xs font-medium ${STATUS_COLORS[status]}`}
    >
      {STATUS_LABELS[status]}
    </span>
  );
}
