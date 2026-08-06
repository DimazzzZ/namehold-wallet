import { useMemo, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { PageHeader } from "./ui/PageHeader";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { Input } from "./ui/Input";
import { useUiStore } from "../stores/ui";
import { auctionPhase, nextTransition, formatCountdown } from "../lib/auction";
import { displayName } from "../lib/idn";
import { formatHns } from "../lib/utils";
import { useReadNames } from "../queries/read";
import type { HsdName } from "../types";
import { NameInfoModal } from "./NameInfoModal";

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
  const [infoName, setInfoName] = useState<string | null>(null);
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

  // Fetch full HsdName info for every watched name. The row uses `state` for
  // the phase badge, `stats` for the countdown / expiry columns, and `highest`
  // for the highest-bid column. Keyed on the joined name list so the query is
  // stable across renders; a 30s staleTime avoids refetching on every mount.
  const watchedNamesKey = watched.map((w) => w.name).join(",");
  const { data: liveInfos = {} } = useQuery<Record<string, HsdName>>({
    queryKey: ["watchlist", "liveInfo", watchedNamesKey],
    queryFn: async () => {
      const names = watched.map((w) => w.name);
      const results = await Promise.all(
        names.map(async (name) => {
          try {
            const raw = await invoke<HsdName | null>("read_name_info", { name });
            return { name, info: raw ?? null };
          } catch {
            return { name, info: null };
          }
        }),
      );
      const out: Record<string, HsdName> = {};
      for (const r of results) {
        if (r.info) out[r.name] = r.info;
      }
      return out;
    },
    enabled: watched.length > 0,
    staleTime: 30_000,
  });

  // Owned-by-active-profile set for the "Owned" badge. Same hook WalletView
  // uses; pinned to the active profile. Zero backend change.
  const { data: ownedList = [] } = useReadNames();
  const ownedNames = useMemo(
    () => new Set(ownedList.map((n) => n.name)),
    [ownedList],
  );

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

  const getInfo = (name: string): HsdName | undefined => liveInfos[name];

  // Days-until-expire color thresholds mirror src/components/Renewals.tsx:31-33.
  const expiryColor = (days: number | null | undefined): string => {
    if (days == null) return "text-gray-500";
    if (days <= 30) return "text-red-600";
    if (days < 90) return "text-yellow-600";
    return "text-green-600";
  };

  const startEditTags = (name: string, currentTags: string) => {
    setEditingTags(name);
    setTagValue(currentTags);
  };

  const saveTags = (name: string) => {
    updateTagsMutation.mutate({ name, tags: tagValue });
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Watchlist"
        subtitle="Track names you don't own — monitor auctions, expiry, and availability."
      />

      {/* Add to watchlist + CSV buttons */}
      <div className="flex items-end gap-2 flex-wrap">
        <Input
          inputSize="md"
          className="w-56"
          value={addName}
          onChange={(e) => setAddName(e.target.value)}
          placeholder="Enter a name to watch"
          onKeyDown={(e) => {
            if (e.key === "Enter") handleAdd();
          }}
          data-testid="watchlist-add-input"
        />
        <Button size="md" variant="primary" onClick={handleAdd} disabled={!addName.trim()}>
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
                  <th className="py-1 pr-4">Countdown</th>
                  <th className="py-1 pr-4">Highest bid</th>
                  <th className="py-1 pr-4">Expires</th>
                  <th className="py-1 pr-4">Tags</th>
                  <th className="py-1 pr-4">Added</th>
                  <th className="py-1"></th>
                </tr>
              </thead>
              <tbody>
              {watched.map((w) => {
                  const state = getState(w.name);
                  const phase = state ? auctionPhase(state) : null;
                  const info = getInfo(w.name);
                  const countdown = info ? nextTransition(state, info.stats) : null;
                  const daysUntilExpire = info?.stats?.daysUntilExpire;
                  const tags = w.tags
                    .split(",")
                    .map((t) => t.trim())
                    .filter(Boolean);
                  return (
                    <tr key={w.name} className="border-t border-gray-100 hover:bg-gray-50">
                      <td className="py-1 pr-4 text-xs font-mono">
                        <button
                          type="button"
                          className="text-blue-500 hover:text-blue-700 hover:underline cursor-pointer"
                          onClick={() => setInfoName(w.name)}
                        >
                          .{displayName(w.name)}
                        </button>
                        {ownedNames.has(w.name) && (
                          <Badge variant="success" className="ml-1">Owned</Badge>
                        )}
                      </td>
                      <td className="py-1 pr-4">
                        {phase ? (
                          <Badge variant={phase.variant}>{phase.label}</Badge>
                        ) : state ? (
                          <span className="text-gray-500 text-xs">Unknown state</span>
                        ) : (
                          <span className="text-gray-500 text-xs">Loading{"\u2026"}</span>
                        )}
                      </td>
                      <td className="py-1 pr-4 text-xs text-gray-600 whitespace-nowrap">
                        {countdown ? (
                          <span title={countdown.label}>
                            {countdown.label}: {formatCountdown(countdown)}
                          </span>
                        ) : (
                          <span className="text-gray-500">{"\u2014"}</span>
                        )}
                      </td>
                      <td className="py-1 pr-4 text-xs font-mono whitespace-nowrap">
                        {info && info.highest != null && info.highest > 0 ? (
                          <span>{formatHns(info.highest)} HNS</span>
                        ) : (
                          <span className="text-gray-500">{"\u2014"}</span>
                        )}
                      </td>
                      <td
                        className={`py-1 pr-4 text-xs whitespace-nowrap ${expiryColor(daysUntilExpire)}`}
                      >
                        {daysUntilExpire != null
                          ? `${Math.floor(daysUntilExpire)}d`
                          : "\u2014"}
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
                              <span className="text-gray-500 text-xs">{"\u2014"}</span>
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

      {infoName && (
        <NameInfoModal
          name={infoName}
          open={!!infoName}
          onClose={() => setInfoName(null)}
        />
      )}
    </div>
  );
}
