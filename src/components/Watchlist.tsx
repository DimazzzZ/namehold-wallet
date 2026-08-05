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
}

interface NameInfo {
  state: string | null;
  height: number | null;
  renewal: number | null;
  registered: boolean | null;
}

export function Watchlist() {
  const [addName, setAddName] = useState("");
  const showToast = useUiStore((s) => s.showToast);
  const qc = useQueryClient();

  // Fetch watchlist from backend.
  const { data: watched = [] } = useQuery<WatchedName[]>({
    queryKey: ["watchlist"],
    queryFn: () => invoke<WatchedName[]>("list_watchlist"),
  });

  // Bulk-fetch name info for all watched names (for state/expiry display).
  const { data: nameInfos = {} } = useQuery<Record<string, NameInfo>>({
    queryKey: ["watchlist", "nameInfos", watched.map((w) => w.name).join(",")],
    queryFn: async () => {
      const out: Record<string, NameInfo> = {};
      for (const w of watched) {
        try {
          const raw = await invoke<Record<string, unknown> | null>("read_name_info", { name: w.name });
          if (raw) {
            out[w.name] = {
              state: (raw.state as string) ?? null,
              height: (raw.height as number) ?? null,
              renewal: (raw.renewal as number) ?? null,
              registered: (raw.registered as boolean) ?? null,
            };
          }
        } catch {
          out[w.name] = { state: null, height: null, renewal: null, registered: null };
        }
      }
      return out;
    },
    enabled: watched.length > 0,
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

  const handleAdd = () => {
    const name = addName.trim();
    if (!name) return;
    addMutation.mutate(name);
  };

  return (
    <div className="space-y-6 max-w-3xl">
      <PageHeader
        title="Watchlist"
        subtitle="Track names you don't own — monitor auctions, expiry, and availability."
      />

      {/* Add to watchlist */}
      <div className="flex items-end gap-2">
        <input
          className="border border-gray-300 rounded px-2 py-1.5 text-sm w-56"
          value={addName}
          onChange={(e) => setAddName(e.target.value)}
          placeholder="Enter a name to watch"
          onKeyDown={(e) => { if (e.key === "Enter") handleAdd(); }}
          data-testid="watchlist-add-input"
        />
        <Button size="sm" variant="primary" onClick={handleAdd} disabled={!addName.trim()}>
          Add
        </Button>
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
                  <th className="py-1 pr-4">Added</th>
                  <th className="py-1"></th>
                </tr>
              </thead>
              <tbody>
                {watched.map((w) => {
                  const info = nameInfos[w.name];
                  const phase = info?.state ? auctionPhase(info.state) : null;
                  return (
                    <tr key={w.name} className="border-t border-gray-100 hover:bg-gray-50">
                      <td className="py-1 pr-4 text-xs font-mono">
                        .{displayName(w.name)}
                      </td>
                      <td className="py-1 pr-4">
                        {phase ? (
                          <Badge variant={phase.variant}>{phase.label}</Badge>
                        ) : info ? (
                          <span className="text-gray-400 text-xs">Unknown state</span>
                        ) : (
                          <span className="text-gray-400 text-xs">Loading…</span>
                        )}
                      </td>
                      <td className="py-1 pr-4 text-xs text-gray-500">
                        {w.addedAt ? new Date(w.addedAt).toLocaleDateString() : "—"}
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
