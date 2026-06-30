import { useState } from "react";
import { Link } from "react-router-dom";
import { useBatches, useCreateBatch, useDeleteBatch, useBatchWithAssets, useUpdateBatch, useRemoveFromBatch } from "../queries/batches";
import { useAssets } from "../queries/assets";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { Dialog } from "./ui/Dialog";
import { Badge } from "./ui/Badge";
import { Select } from "./ui/Select";
import { StatusBadge } from "./ui/StatusBadge";
import { formatDate } from "../lib/utils";
import { useUiStore } from "../stores/ui";
import type { BatchStatus, MigrationStatus } from "../types";

const STATUS_VARIANTS: Record<BatchStatus, "default" | "info" | "success" | "warning" | "error"> = {
  planned: "default",
  in_progress: "info",
  completed: "success",
  paused: "warning",
  cancelled: "error",
};

const BATCH_STATUS_OPTIONS = [
  { value: "planned", label: "Planned" },
  { value: "in_progress", label: "In Progress" },
  { value: "completed", label: "Completed" },
  { value: "paused", label: "Paused" },
  { value: "cancelled", label: "Cancelled" },
];

export function Batches() {
  const { data: batches = [], isLoading } = useBatches();
  const createBatch = useCreateBatch();
  const deleteBatch = useDeleteBatch();
  const updateBatch = useUpdateBatch();
  const removeFromBatch = useRemoveFromBatch();
  const { showToast } = useUiStore();
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newDesc, setNewDesc] = useState("");
  const [newBatchAssetIds, setNewBatchAssetIds] = useState<Set<number>>(new Set());
  const [selectedBatchId, setSelectedBatchId] = useState<number | null>(null);
  const [addTldDialogOpen, setAddTldDialogOpen] = useState(false);

  const { data: batchDetail } = useBatchWithAssets(selectedBatchId ?? 0);
  const { data: allAssets = [] } = useAssets({});

  const handleCreate = async () => {
    if (!newName.trim()) return;
    try {
      await createBatch.mutateAsync({ name: newName, description: newDesc || undefined, asset_ids: Array.from(newBatchAssetIds) });
      showToast(`Batch created with ${newBatchAssetIds.size} TLDs`, "success");
      setCreateDialogOpen(false);
      setNewName("");
      setNewDesc("");
      setNewBatchAssetIds(new Set());
    } catch (e) {
      showToast(`Failed: ${e}`, "error");
    }
  };

  const handleDelete = async (id: number) => {
    if (!confirm("Delete this batch?")) return;
    try {
      await deleteBatch.mutateAsync(id);
      showToast("Batch deleted", "success");
      if (selectedBatchId === id) setSelectedBatchId(null);
    } catch (e) {
      showToast(`Failed: ${e}`, "error");
    }
  };

  const handleStatusChange = async (batchId: number, status: string) => {
    try {
      await updateBatch.mutateAsync({ id: batchId, status });
      showToast("Status updated", "success");
    } catch (e) {
      showToast(`Failed: ${e}`, "error");
    }
  };

  const handleRemoveFromBatch = async (batchId: number, assetIds: number[]) => {
    try {
      await removeFromBatch.mutateAsync({ batch_id: batchId, asset_ids: assetIds });
      showToast(`Removed ${assetIds.length} TLD(s)`, "success");
    } catch (e) {
      showToast(`Failed: ${e}`, "error");
    }
  };

  const handleAddToBatch = async (assetIds: number[]) => {
    if (!selectedBatchId) return;
    try {
      const { invoke } = await import("../lib/invoke");
      await invoke("add_to_batch", { batch_id: selectedBatchId, asset_ids: assetIds });
      showToast(`Added ${assetIds.length} TLD(s)`, "success");
      setAddTldDialogOpen(false);
    } catch (e) {
      showToast(`Failed: ${e}`, "error");
    }
  };

  if (selectedBatchId && batchDetail) {
    return (
      <div className="space-y-4">
        <div className="flex items-center gap-3">
          <Button size="sm" variant="ghost" onClick={() => setSelectedBatchId(null)}>
            &larr; Back
          </Button>
          <h2 className="text-xl font-bold">{batchDetail.name}</h2>
          <Badge variant={STATUS_VARIANTS[batchDetail.status]}>{batchDetail.status}</Badge>
        </div>

        {batchDetail.description && (
          <p className="text-sm text-gray-600">{batchDetail.description}</p>
        )}

        <div className="flex gap-3 items-end">
          <Select
            label="Status"
            options={BATCH_STATUS_OPTIONS}
            value={batchDetail.status}
            onChange={(e) => handleStatusChange(selectedBatchId, e.target.value)}
          />
          <Button size="sm" onClick={() => setAddTldDialogOpen(true)}>
            Add TLDs
          </Button>
        </div>

        {batchDetail.assets.length === 0 ? (
          <div className="text-gray-500 bg-white rounded p-8 border text-center">
            No TLDs in this batch. Click "Add TLDs" to add some.
          </div>
        ) : (
          <div className="bg-white rounded border border-gray-200">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="px-3 py-2">TLD</th>
                  <th className="px-3 py-2">Status</th>
                  <th className="px-3 py-2">Category</th>
                  <th className="px-3 py-2">Notes</th>
                  <th className="px-3 py-2"></th>
                </tr>
              </thead>
              <tbody>
                {batchDetail.assets.map((asset) => (
                  <tr key={asset.id} className="border-t border-gray-100">
                    <td className="px-3 py-2 font-mono">.{asset.tld}</td>
                    <td className="px-3 py-2">
                      <StatusBadge status={asset.status as MigrationStatus} />
                    </td>
                    <td className="px-3 py-2 text-gray-500">{asset.category || "—"}</td>
                    <td className="px-3 py-2 text-gray-500 truncate max-w-[200px]">{asset.notes || "—"}</td>
                    <td className="px-3 py-2">
                      <Button
                        size="sm"
                        variant="danger"
                        onClick={() => handleRemoveFromBatch(selectedBatchId, [asset.id])}
                      >
                        Remove
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        <Dialog open={addTldDialogOpen} onClose={() => setAddTldDialogOpen(false)} title="Add TLDs to Batch">
          <div className="max-h-60 overflow-auto space-y-1">
            {allAssets
              .filter((a) => !batchDetail.assets.some((ba) => ba.id === a.id))
              .map((asset) => (
                <div
                  key={asset.id}
                  className="flex items-center justify-between py-1 px-2 hover:bg-gray-50 rounded cursor-pointer"
                  onClick={() => handleAddToBatch([asset.id])}
                >
                  <span className="font-mono text-sm">.{asset.tld}</span>
                  <StatusBadge status={asset.status as MigrationStatus} />
                </div>
              ))}
          </div>
        </Dialog>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-bold">Batches</h2>
        <Button variant="primary" size="sm" onClick={() => setCreateDialogOpen(true)}>
          New Batch
        </Button>
      </div>

      {isLoading ? (
        <div className="text-gray-500">Loading...</div>
      ) : batches.length === 0 ? (
        <div className="bg-white rounded p-8 border text-center">
          <div className="text-gray-500 mb-3">No batches created yet.</div>
          <div className="text-sm text-gray-400 mb-4">
            Batches help you organize TLDs into migration groups.
          </div>
          <div className="flex gap-2 justify-center">
            <Link to="/inventory">
              <Button size="sm">Select TLDs in Inventory</Button>
            </Link>
            <Button size="sm" variant="primary" onClick={() => setCreateDialogOpen(true)}>
              New Batch
            </Button>
          </div>
        </div>
      ) : (
        <div className="space-y-2">
          {batches.map((batch) => (
            <div
              key={batch.id}
              className="bg-white rounded border border-gray-200 p-4 flex items-center justify-between cursor-pointer hover:bg-gray-50"
              onClick={() => setSelectedBatchId(batch.id)}
            >
              <div>
                <div className="font-medium">{batch.name}</div>
                {batch.description && (
                  <div className="text-sm text-gray-500">{batch.description}</div>
                )}
                <div className="flex gap-2 mt-1">
                  <Badge variant={STATUS_VARIANTS[batch.status]}>{batch.status}</Badge>
                  <span className="text-xs text-gray-400">
                    {batch.asset_count ?? 0} TLDs
                  </span>
                  <span className="text-xs text-gray-400">
                    {formatDate(batch.created_at)}
                  </span>
                </div>
              </div>
              <Button
                variant="danger"
                size="sm"
                onClick={(e) => {
                  e.stopPropagation();
                  handleDelete(batch.id);
                }}
              >
                Delete
              </Button>
            </div>
          ))}
        </div>
      )}

      <Dialog open={createDialogOpen} onClose={() => setCreateDialogOpen(false)} title="New Batch">
        <div className="space-y-3">
          <Input
            label="Batch Name"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder="e.g. Test Batch 1"
          />
          <Input
            label="Description (optional)"
            value={newDesc}
            onChange={(e) => setNewDesc(e.target.value)}
            placeholder="Migration test batch"
          />
          <div>
            <label className="text-sm font-medium text-gray-700">Select TLDs (optional)</label>
            <div className="mt-1 border border-gray-300 rounded max-h-40 overflow-auto">
              {allAssets.length === 0 ? (
                <div className="px-3 py-2 text-sm text-gray-400">No TLDs available</div>
              ) : (
                allAssets.map((asset) => (
                  <label
                    key={asset.id}
                    className="flex items-center gap-2 px-3 py-1 hover:bg-gray-50 cursor-pointer"
                  >
                    <input
                      type="checkbox"
                      checked={newBatchAssetIds.has(asset.id)}
                      onChange={() => {
                        setNewBatchAssetIds((prev) => {
                          const next = new Set(prev);
                          if (next.has(asset.id)) {
                            next.delete(asset.id);
                          } else {
                            next.add(asset.id);
                          }
                          return next;
                        });
                      }}
                    />
                    <span className="font-mono text-sm">.{asset.tld}</span>
                    <StatusBadge status={asset.status as MigrationStatus} />
                  </label>
                ))
              )}
            </div>
            {newBatchAssetIds.size > 0 && (
              <div className="text-xs text-gray-500 mt-1">{newBatchAssetIds.size} TLDs selected</div>
            )}
          </div>
          <div className="flex gap-2 justify-end">
            <Button variant="ghost" onClick={() => setCreateDialogOpen(false)}>
              Cancel
            </Button>
            <Button variant="primary" onClick={handleCreate} disabled={!newName.trim()}>
              Create
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}
