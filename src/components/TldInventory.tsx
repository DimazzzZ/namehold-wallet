import { useState, useCallback } from "react";
import { type ColumnDef } from "@tanstack/react-table";
import { useQueryClient } from "@tanstack/react-query";
import { useAssets, useBulkUpdateStatus, useUpdateAsset, useImportCsv, useExportCsv } from "../queries/assets";
import { useCreateBatch } from "../queries/batches";
import { useTransferName } from "../queries/wallet";
import { useSettingsStore } from "../stores/settings";
import { useUiStore } from "../stores/ui";
import { DataTable } from "./ui/DataTable";
import { StatusBadge } from "./ui/StatusBadge";
import { Button } from "./ui/Button";
import { Select } from "./ui/Select";
import { Dialog } from "./ui/Dialog";
import { Input } from "./ui/Input";
import type { Asset, MigrationStatus } from "../types";
import { formatHns, formatDate } from "../lib/utils";
import { mapError } from "../lib/errors";
import { open, save } from "@tauri-apps/plugin-dialog";
import { invoke } from "../lib/invoke";

const MIGRATION_STATUSES: { value: string; label: string }[] = [
  { value: "", label: "All Statuses" },
  { value: "not_started", label: "Not Started" },
  { value: "namebase_transfer_requested", label: "Transfer Requested" },
  { value: "waiting_transfer_tx", label: "Waiting TX" },
  { value: "transfer_seen_on_chain", label: "TX Seen" },
  { value: "waiting_finalize", label: "Waiting Finalize" },
  { value: "finalized_owned", label: "Finalized" },
  { value: "failed_or_stuck", label: "Failed/Stuck" },
  { value: "do_not_touch_staked", label: "Do Not Touch" },
];

const TAG_PRESETS = ["high_value", "medium_value", "low_value", "test", "operational", "trash"];

export function TldInventory() {
  const [statusFilter, setStatusFilter] = useState("");
  const [stakedFilter, setStakedFilter] = useState<"all" | "staked" | "unstaked">("all");
  const [sortBy, setSortBy] = useState("tld");
  const [sortDir, setSortDir] = useState<"asc" | "desc">("asc");
  const [editingNotes, setEditingNotes] = useState<{ id: number; notes: string } | null>(null);
  const [bulkStatusDialog, setBulkStatusDialog] = useState(false);
  const [bulkTagDialog, setBulkTagDialog] = useState(false);
  const [batchDialogOpen, setBatchDialogOpen] = useState(false);
  const [batchName, setBatchName] = useState("");
  const [transferDialogOpen, setTransferDialogOpen] = useState(false);
  const [transferAddress, setTransferAddress] = useState("");
  const [transferPassphrase, setTransferPassphrase] = useState("");
  const [transferConfirmName, setTransferConfirmName] = useState("");

  const { selectedAssetIds, clearSelection, showToast } =
    useUiStore();
  const settings = useSettingsStore((s) => s.settings);
  const passphrase = useSettingsStore((s) => s.passphrase);
  const writeMode = settings?.write_mode === "true";
  const transferName = useTransferName();
  const qc = useQueryClient();

  const params = {
    status: statusFilter || undefined,
    is_staked: stakedFilter === "all" ? undefined : stakedFilter === "staked",
    sort_by: sortBy,
    sort_dir: sortDir,
  };

  const { data: assets = [], isLoading } = useAssets(params);
  const bulkUpdateStatus = useBulkUpdateStatus();
  const updateAsset = useUpdateAsset();
  const importCsv = useImportCsv();
  const exportCsv = useExportCsv();
  const createBatch = useCreateBatch();

  const handleImport = useCallback(async () => {
    const selected = await open({
      multiple: false,
      filters: [{ name: "CSV", extensions: ["csv"] }],
    });
    if (!selected) return;
    try {
      const result = await importCsv.mutateAsync(selected as string);
      showToast(
        `Imported ${result.imported} TLDs${result.errors.length > 0 ? `, ${result.errors.length} errors` : ""}`,
        result.errors.length > 0 ? "error" : "success",
      );
    } catch (e) {
      showToast(`Import failed: ${e}`, "error");
    }
  }, [importCsv, showToast]);

  const handleExport = useCallback(async () => {
    const path = await save({
      filters: [{ name: "CSV", extensions: ["csv"] }],
      defaultPath: "hns-portfolio-export.csv",
    });
    if (!path) return;
    try {
      const count = await exportCsv.mutateAsync({ path });
      showToast(`Exported ${count} TLDs`, "success");
    } catch (e) {
      showToast(`Export failed: ${e}`, "error");
    }
  }, [exportCsv, showToast]);

  const handleBulkStatus = useCallback(
    async (status: string) => {
      const ids = Array.from(selectedAssetIds);
      if (ids.length === 0) return;
      try {
        await bulkUpdateStatus.mutateAsync({ ids, status });
        showToast(`Updated ${ids.length} TLDs to ${status}`, "success");
        clearSelection();
        setBulkStatusDialog(false);
      } catch (e) {
        showToast(`Bulk update failed: ${e}`, "error");
      }
    },
    [selectedAssetIds, bulkUpdateStatus, showToast, clearSelection],
  );

  const handleSaveNotes = useCallback(
    async (id: number, notes: string) => {
      try {
        await updateAsset.mutateAsync({ id, notes });
        setEditingNotes(null);
      } catch (e) {
        showToast(`Failed to save notes: ${e}`, "error");
      }
    },
    [updateAsset, showToast],
  );

  const columns: ColumnDef<Asset, unknown>[] = [
    {
      accessorKey: "tld",
      header: "TLD",
      size: 160,
      cell: (info) => (
        <span className="font-mono font-semibold text-sm">
          .{info.getValue<string>()}
        </span>
      ),
    },
    {
      accessorKey: "status",
      header: "Status",
      size: 180,
      cell: (info) => <StatusBadge status={info.getValue<MigrationStatus>()} />,
    },
    {
      accessorKey: "is_staked",
      header: "Staked",
      size: 70,
      cell: (info) => (info.getValue<boolean>() ? "S" : ""),
    },
    {
      accessorKey: "category",
      header: "Category",
      size: 120,
      cell: (info) => info.getValue<string>() || "—",
    },
    {
      accessorKey: "name_state",
      header: "HNS State",
      size: 110,
      cell: (info) => info.getValue<string>() || "—",
    },
    {
      accessorKey: "hns_received",
      header: "HNS",
      size: 110,
      cell: (info) => formatHns(info.getValue<number | null>()),
    },
    {
      accessorKey: "expires_at_height",
      header: "Expires",
      size: 100,
      cell: (info) => {
        const v = info.getValue<number | null>();
        return v != null ? `#${v}` : "—";
      },
    },
    {
      accessorKey: "notes",
      header: "Notes",
      size: 200,
      cell: (info) => {
        const asset = info.row.original;
        if (editingNotes?.id === asset.id) {
          return (
            <input
              className="border rounded px-1 py-0.5 text-xs w-full"
              value={editingNotes.notes}
              onChange={(e) =>
                setEditingNotes({ id: asset.id, notes: e.target.value })
              }
              onBlur={() => handleSaveNotes(asset.id, editingNotes.notes)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleSaveNotes(asset.id, editingNotes.notes);
                if (e.key === "Escape") setEditingNotes(null);
              }}
              autoFocus
            />
          );
        }
        const notes = info.getValue<string | null>();
        return (
          <span
            className="text-xs text-gray-500 truncate block max-w-[200px] cursor-text"
            onClick={(e) => {
              e.stopPropagation();
              setEditingNotes({ id: asset.id, notes: notes || "" });
            }}
          >
            {notes || "—"}
          </span>
        );
      },
    },
    {
      accessorKey: "updated_at",
      header: "Updated",
      size: 140,
      cell: (info) => (
        <span className="text-xs text-gray-400">
          {formatDate(info.getValue<string>())}
        </span>
      ),
    },
  ];

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-bold">TLD Inventory</h2>
        <div className="flex gap-2">
          <Button onClick={handleImport} variant="primary" size="sm">
            Import CSV
          </Button>
          <Button
            onClick={async () => {
              try {
                const result: any = await invoke("import_from_namebase");
                showToast(
                  `Imported ${result.imported} TLDs from Namebase (${result.staked_count} staked)`,
                  "success",
                );
                qc.invalidateQueries({ queryKey: ["assets"] });
              } catch (e) {
                showToast(mapError(e), "error");
              }
            }}
            size="sm"
          >
            Import from Namebase
          </Button>
          <Button onClick={handleExport} size="sm">
            Export CSV
          </Button>
        </div>
      </div>

      <div className="flex gap-3 items-end">
        <Select
          options={[
            { value: "all", label: "All" },
            { value: "staked", label: "Staked" },
            { value: "unstaked", label: "Unstaked" },
          ]}
          value={stakedFilter}
          onChange={(e) => setStakedFilter(e.target.value as "all" | "staked" | "unstaked")}
        />
        <Select
          options={MIGRATION_STATUSES}
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value)}
        />
        <Select
          options={[
            { value: "tld", label: "Name" },
            { value: "status", label: "Status" },
            { value: "category", label: "Category" },
            { value: "updated_at", label: "Updated" },
          ]}
          value={sortBy}
          onChange={(e) => setSortBy(e.target.value)}
        />
        <Button
          size="sm"
          onClick={() => setSortDir((d) => (d === "asc" ? "desc" : "asc"))}
        >
          {sortDir === "asc" ? "ASC" : "DESC"}
        </Button>
      </div>

      {selectedAssetIds.size > 0 && (
        <div className="flex items-center gap-3 bg-blue-50 border border-blue-200 rounded px-3 py-2">
          <span className="text-sm text-blue-700">{selectedAssetIds.size} selected</span>
          <Button size="sm" onClick={() => setBulkStatusDialog(true)}>
            Update Status
          </Button>
          <Button size="sm" onClick={() => setBulkTagDialog(true)}>
            Set Tags
          </Button>
          <Button size="sm" onClick={() => setBatchDialogOpen(true)}>
            Create Batch
          </Button>
          {writeMode && (
            <Button size="sm" variant="danger" onClick={() => setTransferDialogOpen(true)}>
              Transfer {selectedAssetIds.size > 1 ? `(${selectedAssetIds.size})` : ""}
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={clearSelection}>
            Clear
          </Button>
        </div>
      )}

      {isLoading ? (
        <div className="text-gray-500">Loading...</div>
      ) : assets.length === 0 ? (
        <div className="bg-white rounded p-8 border text-center">
          <div className="text-gray-500 mb-3">No TLDs imported yet.</div>
          <div className="text-sm text-gray-400 mb-4">
            Import a CSV file with your TLDs to get started.
          </div>
          <Button variant="primary" onClick={handleImport}>
            Import CSV
          </Button>
        </div>
      ) : (
        <DataTable
          data={assets}
          columns={columns}
          enableRowSelection
          selectedIds={selectedAssetIds}
          onSelectionChange={(ids) => {
            useUiStore.setState({ selectedAssetIds: ids });
          }}
        />
      )}

      <Dialog
        open={bulkStatusDialog}
        onClose={() => setBulkStatusDialog(false)}
        title="Update Status"
      >
        <p className="text-sm text-gray-600 mb-3">
          Update {selectedAssetIds.size} TLDs to:
        </p>
        <div className="space-y-2">
          {MIGRATION_STATUSES.filter((s) => s.value).map((s) => (
            <Button
              key={s.value}
              variant="secondary"
              className="w-full justify-start"
              onClick={() => handleBulkStatus(s.value)}
            >
              {s.label}
            </Button>
          ))}
        </div>
      </Dialog>

      <Dialog
        open={bulkTagDialog}
        onClose={() => setBulkTagDialog(false)}
        title="Set Tags"
      >
        <p className="text-sm text-gray-600 mb-3">
          Set tags for {selectedAssetIds.size} TLDs:
        </p>
        <div className="space-y-2">
          {TAG_PRESETS.map((tag) => (
            <Button
              key={tag}
              variant="secondary"
              className="w-full justify-start"
              onClick={async () => {
                const ids = Array.from(selectedAssetIds);
                try {
                  await updateAsset.mutateAsync({
                    id: ids[0]!,
                    tags: JSON.stringify([tag]),
                  });
                  showToast(`Set tag "${tag}" on ${ids.length} TLDs`, "success");
                  clearSelection();
                  setBulkTagDialog(false);
                } catch (e) {
                  showToast(`Failed: ${e}`, "error");
                }
              }}
            >
              {tag.replace("_", " ")}
            </Button>
          ))}
        </div>
      </Dialog>

      <Dialog
        open={batchDialogOpen}
        onClose={() => setBatchDialogOpen(false)}
        title="Create Batch from Selection"
      >
        <div className="space-y-3">
          <p className="text-sm text-gray-600">
            Create a batch from {selectedAssetIds.size} selected TLDs.
          </p>
          <Input
            label="Batch Name"
            value={batchName}
            onChange={(e) => setBatchName(e.target.value)}
            placeholder="e.g. Test Batch 1"
          />
          <div className="flex gap-2 justify-end">
            <Button variant="ghost" onClick={() => setBatchDialogOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={!batchName.trim()}
              onClick={async () => {
                const ids = Array.from(selectedAssetIds);
                try {
                  await createBatch.mutateAsync({ name: batchName, asset_ids: ids });
                  showToast(`Created batch "${batchName}" with ${ids.length} TLDs`, "success");
                  clearSelection();
                  setBatchDialogOpen(false);
                  setBatchName("");
                } catch (e) {
                  showToast(`Failed: ${e}`, "error");
                }
              }}
            >
              Create
            </Button>
          </div>
        </div>
      </Dialog>

      <Dialog
        open={transferDialogOpen}
        onClose={() => setTransferDialogOpen(false)}
        title="Transfer TLD"
      >
        <div className="space-y-3">
          <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
            This will initiate a name transfer on-chain. The TLD will be sent to the specified address.
            This action cannot be undone.
          </div>
          {(() => {
            const tld = assets.find(a => selectedAssetIds.has(a.id))?.tld;
            const confirmMatch = transferConfirmName === tld;
            return (
              <>
                <p className="text-sm text-gray-600">
                  Transferring: <strong>.{tld}</strong>
                </p>
                <Input
                  label="Destination Address"
                  value={transferAddress}
                  onChange={(e) => setTransferAddress(e.target.value)}
                  placeholder="hs1q..."
                />
                <Input
                  label="Wallet Passphrase"
                  type="password"
                  value={transferPassphrase}
                  onChange={(e) => setTransferPassphrase(e.target.value)}
                  placeholder={passphrase ? "Using saved passphrase" : "Enter passphrase"}
                />
                <Input
                  label={`Type "${tld}" to confirm`}
                  value={transferConfirmName}
                  onChange={(e) => setTransferConfirmName(e.target.value)}
                  placeholder={tld}
                />
                <div className="flex gap-2 justify-end">
                  <Button variant="ghost" onClick={() => { setTransferDialogOpen(false); setTransferConfirmName(""); }}>
                    Cancel
                  </Button>
                  <Button
                    variant="danger"
                    disabled={!transferAddress.trim() || !confirmMatch || transferName.isPending}
                    onClick={async () => {
                      const id = Array.from(selectedAssetIds)[0];
                      const asset = assets.find(a => a.id === id);
                      if (!asset) return;
                      const pw = transferPassphrase || passphrase;
                      if (!pw) {
                        showToast("Enter wallet passphrase in Settings or below", "error");
                        return;
                      }
                      try {
                        await transferName.mutateAsync({ name: asset.tld, address: transferAddress, passphrase: pw });
                        showToast(`Transfer initiated for .${asset.tld}`, "success");
                        clearSelection();
                        setTransferDialogOpen(false);
                        setTransferAddress("");
                        setTransferPassphrase("");
                        setTransferConfirmName("");
                      } catch (e) {
                        showToast(mapError(e), "error");
                      }
                    }}
                  >
                    {transferName.isPending ? "Transferring..." : "Transfer"}
                  </Button>
                </div>
              </>
            );
          })()}
        </div>
      </Dialog>
    </div>
  );
}
