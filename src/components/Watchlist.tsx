import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { PageHeader } from "./ui/PageHeader";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { useUiStore } from "../stores/ui";
import { auctionPhase } from "../lib/auction";
import { displayName } from "../lib/idn";

interface WatchedName {
  name: string;
  addedAt: string;
  notes: string;
  tags: string;
}

interface WatchlistStatus {
  name: string;
  watched: boolean;
  tags: string;
  state: string | null;
  expiry: number | null;
}

export function Watchlist() {
  const [addName, setAddName] = useState("");
  const [editingTags, setEditingTags] = useState<string | null>(null);
  const [tagValue, setTagValue] = useState("");
  const showToast = useUiStore((s) => s.showToast);
  const qc = useQueryClient();

  // Fetch watchlist from backend.
  const { data: watched = [] } = useQuery<WatchedName[]>({
    queryKey: ["watchlist"],
    queryFn: () => invoke<WatchedName[]>("list_watchlist"),
  });

  // Bulk-fetch watchlist status (membership + cached state) for all watched names.
  const { data: statuses = [] } = useQuery<WatchlistStatus[]>({
    queryKey: ["watchlist", "status", watched.map((w) => w.name).join(",")],
    queryFn: () =>
      invoke<WatchlistStatus[]>("get_watchlist_status", {
        names: watched.map((w) => w.name),
      }),
    enabled: watched.length > 0,
    staleTime: 30_000,
  });

  // For names where get_watchlist_status returned state=null, fetch live info.
  const namesNeedingLiveState = watched
    .map((w) => w.name)
    .filter((n) => {
      const s = statuses.find((st) => st.name === n);
      return !s || s.state === null;
    });

  const { data: liveInfos = {} } = useQuery<Record<string, { state: string | null }>>({
    queryKey: ["watchlist", "liveInfo", namesNeedingLiveState.join(",")],
    queryFn: async () => {
      const results = await Promise.all(
        namesNeedingLiveState.map(async (name) => {
          try {
            const raw = await invoke<Record<string, unknown> | null>("read_name_info", { name });
            return { name, state: (raw?.state as string) ?? null };
          } catch {
            return { name, state: null };
          }
        }),
      );
      const out: Record<string, { state: string | null }> = {};
      for (const r of results) out[r.name] = { state: r.state };
      return out;
    },
    enabled: namesNeedingLiveState.length > 0,
    staleTime: 30_000,
  });

  const addMutation = useMutation({
    mutationFn: (name: string) => invoke("add_to_watchlist", { name }),
    onSuccess: (_, name) => {
      setAddName("");
      qc.invalidateQueries({ queryKey: ["watchlist"] });
      showToast(`Added ${name} to watchlist`, "success");
    },
    onError: (err, name) => {
      showToast(`Failed to add ${name}: ${err}`, "error");
    },
  });

  const removeMutation = useMutation({
    mutationFn: (name: string) => invoke("remove_from_watchlist", { name }),
    onSuccess: (_, name) => {
      qc.invalidateQueries({ queryKey: ["watchlist"] });
      showToast(`Removed ${name} from watchlist`, "info");
    },
    onError: (err, name) => {
      showToast(`Failed to remove ${name}: ${err}`, "error");
    },
  });

  const updateTagsMutation = useMutation({
    mutationFn: ({ name, tags }: { name: string; tags: string }) =>
      invoke("update_watchlist_tags", { name, tags }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["watchlist"] });
      setEditingTags(null);
    },
    onError: (err) => {
      showToast(`Failed to update tags: ${err}`, "error");
    },
  });

  const handleAdd = () => {
    const name = addName.trim();
    if (!name) return;
    addMutation.mutate(name);
  };

  const handleExportCsv = async () => {
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        defaultPath: "watchlist-export.csv",
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (!path) return;
      const count = await invoke<number>("export_watchlist_csv", { path });
      showToast(`Exported ${count} name(s) to CSV`, "success");
    } catch (e) {
      showToast(`Export failed: ${e}`, "error");
    }
  };

  const handleImportCsv = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        filters: [{ name: "CSV", extensions: ["csv"] }],
        multiple: false,
      });
      if (!selected) return;
      const result = await invoke<{ imported: number; skipped: number; errors: string[] }>(
        "import_watchlist_csv",
        { path: selected },
      );
      qc.invalidateQueries({ queryKey: ["watchlist"] });
      showToast(
        `Imported ${result.imported}, skipped ${result.skipped}${
          result.errors.length > 0 ? `, ${result.errors.length} error(s)` : ""
        }`,
        "success",
      );
    } catch (e) {
      showToast(`Import failed: ${e}`, "error");
    }
  };

  const getState = (name: string): string | null => {
    const s = statuses.find((st) => st.name === name);
    if (s?.state) return s.state;
    return liveInfos[name]?.state ?? null;
  };

  const startEditTags = (name: string, currentTags: string) => {
    setEditingTags(name);
    setTagValue(currentTags);
  };

  const saveTags = (name: string) => {
    updateTagsMutation.mutate({ name, tags: tagValue });
  };

  return (
    <div className="space-y-6 max-w-3xl">
      <PageHeader
        title="Watchlist"
        subtitle="Track names you don't own \u2014 monitor auctions, expiry, and availability."
      />

      {/* Add to watchlist + CSV buttons */}
      <div className="flex items-end gap-2 flex-wrap">
        <input
          className="border border-gray-300 rounded px-2 py-1.5 text-sm w-56"
          value={addName}
          onChange={(e) => setAddName(e.target.value)}
          placeholder="Enter a name to watch"
          onKeyDown={(e) => {
            if (e.key === "Enter") handleAdd();
          }}
          data-testid="watchlist-add-input"
        />
        <Button size="sm" variant="primary" onClick={handleAdd} disabled={!addName.trim()}>
          Add
        </Button>
        <div className="ml-auto flex gap-2">
          <Button size="sm" variant="ghost" onClick={handleImportCsv}>
            Import CSV
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={handleExportCsv}
            disabled={watched.length === 0}
          >
            Export CSV
          </Button>
        </div>
      </div>

      {/* Watchlist table */}
      {watched.length === 0 ? (
        <div className="text-gray-500 text-sm py-8 text-center">
          No names on your watchlist yet. Enter a name above to start tracking it.
        </div>
      ) : (
        <div className="bg-white rounded p-4 border border-gray-200">
          <div className="flex items-center justify-between mb-2">
            <div className="text-sm text-gray-500">
              Watching {watched.length} name{watched.length !== 1 ? "s" : ""}
            </div>
          </div>
          <div className="max-h-96 overflow-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-500 border-b">
                  <th className="py-1 pr-4">Name</th>
                  <th className="py-1 pr-4">State</th>
                  <th className="py-1 pr-4">Tags</th>
                  <th className="py-1 pr-4">Added</th>
                  <th className="py-1"></th>
                </tr>
              </thead>
              <tbody>
                {watched.map((w) => {
                  const state = getState(w.name);
                  const phase = state ? auctionPhase(state) : null;
                  const tags = w.tags
                    .split(",")
                    .map((t) => t.trim())
                    .filter(Boolean);
                  return (
                    <tr key={w.name} className="border-t border-gray-100 hover:bg-gray-50">
                      <td className="py-1 pr-4 text-xs font-mono">
                        .{displayName(w.name)}
                      </td>
                      <td className="py-1 pr-4">
                        {phase ? (
                          <Badge variant={phase.variant}>{phase.label}</Badge>
                        ) : state ? (
                          <span className="text-gray-400 text-xs">Unknown state</span>
                        ) : (
                          <span className="text-gray-400 text-xs">Loading\u2026</span>
                        )}
                      </td>
                      <td className="py-1 pr-4">
                        {editingTags === w.name ? (
                          <input
                            className="border border-blue-300 rounded px-1 py-0.5 text-xs w-32"
                            value={tagValue}
                            onChange={(e) => setTagValue(e.target.value)}
                            onBlur={() => saveTags(w.name)}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") saveTags(w.name);
                              if (e.key === "Escape") setEditingTags(null);
                            }}
                            autoFocus
                            placeholder="tag1, tag2"
                          />
                        ) : (
                          <button
                            type="button"
                            className="text-left cursor-pointer hover:bg-gray-100 rounded px-1 py-0.5 min-w-[4rem]"
                            onClick={() => startEditTags(w.name, w.tags)}
                            title="Click to edit tags"
                          >
                            {tags.length > 0 ? (
                              <span className="flex flex-wrap gap-1">
                                {tags.map((t) => (
                                  <span
                                    key={t}
                                    className="inline-block bg-gray-100 text-gray-600 text-xs px-1.5 py-0.5 rounded"
                                  >
                                    {t}
                                  </span>
                                ))}
                              </span>
                            ) : (
                              <span className="text-gray-300 text-xs">\u2014</span>
                            )}
                          </button>
                        )}
                      </td>
                      <td className="py-1 pr-4 text-xs text-gray-500">
                        {w.addedAt ? new Date(w.addedAt).toLocaleDateString() : "\u2014"}
                      </td>
                      <td className="py-1 text-right">
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => removeMutation.mutate(w.name)}
                        >
                          Remove
                        </Button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}
