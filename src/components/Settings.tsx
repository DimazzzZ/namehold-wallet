import { useState, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { useSettingsStore } from "../stores/settings";
import { useActiveProfile } from "../queries/wallet";
import { useNodeStatus, useStartHsd, useStopHsd } from "../queries/node";
import { open } from "@tauri-apps/plugin-dialog";
import { Input } from "./ui/Input";
import { Button } from "./ui/Button";
import { StickyFooter } from "./ui/StickyFooter";
import { useUiStore } from "../stores/ui";

/**
 * One coherent settings model for the non-custodial wallet:
 *   - Wallet: the active profile (managed on the Wallet page).
 *   - Connections: the explorer URL (node-free reads) and the hsd node RPC
 *     (needed only to send).
 *   - Advanced (collapsed): address gap limit, signer session timeout, and the
 *     advanced-navigation toggle.
 * No legacy hsd-wallet / connection-mode / write-mode config.
 */
export function Settings() {
  const { settings, loaded, saveAll } = useSettingsStore();
  const showToast = useUiStore((s) => s.showToast);
  const navigate = useNavigate();
  const { data: profile } = useActiveProfile();

  const [form, setForm] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    if (settings) {
      setForm({
        node_rpc_url: settings.node_rpc_url,
        node_rpc_api_key: settings.node_rpc_api_key,
        hsd_prefix: settings.hsd_prefix,
        explorer_api_url: settings.explorer_api_url,
        address_gap_limit: settings.address_gap_limit,
        signer_session_timeout_seconds: settings.signer_session_timeout_seconds,
        advanced_mode: settings.advanced_mode,
      });
    }
  }, [settings]);

  if (!loaded || !settings) {
    return <div className="text-gray-500">Loading settings...</div>;
  }

  const updateField = (key: string, value: string) => {
    setForm((prev) => ({ ...prev, [key]: value }));
    setDirty(true);
  };

  // Pick the hsd data directory with the native folder browser (Finder).
  const pickDataDir = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Choose hsd data directory",
        defaultPath: form.hsd_prefix || undefined,
      });
      if (typeof selected === "string") {
        updateField("hsd_prefix", selected);
      }
    } catch (e) {
      showToast(`Couldn't open folder picker: ${e}`, "error");
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      await saveAll(form);
      setDirty(false);
      showToast("Settings saved", "success");
    } catch (e) {
      showToast(`Failed to save: ${e}`, "error");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-6 max-w-xl pb-16">
      <h2 className="text-xl font-bold">Settings</h2>

      {/* Wallet */}
      <div className="bg-white rounded p-4 border border-gray-200 space-y-3">
        <h3 className="text-sm font-semibold text-gray-700">Wallet</h3>
        {profile ? (
          <div className="text-sm text-gray-600">
            Active: <strong>{profile.label}</strong> · {profile.network}
            {profile.watchOnly ? " · watch-only" : ""}
          </div>
        ) : (
          <div className="text-sm text-gray-500">No wallet profile yet.</div>
        )}
        <Button size="sm" variant="secondary" onClick={() => navigate("/")}>
          Manage wallets
        </Button>
        <div className="bg-blue-50 border border-blue-200 rounded p-2 text-xs text-blue-800">
          Non-custodial: your passphrase is only ever entered in a secure window
          and is never stored by the app.
        </div>
      </div>

      {/* Connections: reads (explorer) + sending (node) in one place. */}
      <div className="bg-white rounded p-4 border border-gray-200 space-y-4">
        <h3 className="text-sm font-semibold text-gray-700">Connections</h3>

        <div className="space-y-2">
          <Input
            label="Explorer URL (reads)"
            value={form.explorer_api_url ?? ""}
            onChange={(e) => updateField("explorer_api_url", e.target.value)}
            placeholder="https://e.hnsfans.com"
          />
          <div className="text-xs text-gray-500">
            Balance and names are read from this explorer — no node required.
          </div>
        </div>

        <div className="space-y-2 pt-2 border-t border-gray-100">
          <Input
            label="Node RPC URL (sending)"
            value={form.node_rpc_url ?? ""}
            onChange={(e) => updateField("node_rpc_url", e.target.value)}
            placeholder="http://127.0.0.1:12037"
          />
          <Input
            label="Node RPC API key"
            type="password"
            value={form.node_rpc_api_key ?? ""}
            onChange={(e) => updateField("node_rpc_api_key", e.target.value)}
            placeholder="(optional)"
          />
          <div className="text-xs text-gray-500">
            Needed only to send or do name actions. Run hsd with{" "}
            <code>--index-address</code>. See NODE_SETUP.md.
          </div>
        </div>

        <div className="space-y-2 pt-2 border-t border-gray-100">
          <div className="flex items-end gap-2">
            <div className="flex-1">
              <Input
                label="Node data directory (hsd --prefix)"
                value={form.hsd_prefix ?? ""}
                onChange={(e) => updateField("hsd_prefix", e.target.value)}
                placeholder="(default: ~/.hsd)"
              />
            </div>
            <Button size="sm" variant="secondary" onClick={pickDataDir}>
              Browse…
            </Button>
          </div>
          <div className="text-xs text-gray-500">
            Where hsd stores the chain. Point this at e.g.{" "}
            <code>/Volumes/WD/hsd-data</code> to keep the large chain off your home
            disk. Empty uses hsd's default (<code>~/.hsd</code>).
          </div>
          <NodeControl dirty={dirty} />
        </div>
      </div>

      {/* Advanced (collapsed by default — rarely changed). */}
      <details className="bg-white rounded border border-gray-200 group">
        <summary className="cursor-pointer select-none px-4 py-3 text-sm font-semibold text-gray-700">
          Advanced
        </summary>
        <div className="px-4 pb-4 space-y-3">
          <Input
            label="Address gap limit"
            value={form.address_gap_limit ?? ""}
            onChange={(e) => updateField("address_gap_limit", e.target.value)}
            placeholder="20"
          />
          <Input
            label="Signer session timeout (seconds)"
            value={form.signer_session_timeout_seconds ?? ""}
            onChange={(e) => updateField("signer_session_timeout_seconds", e.target.value)}
            placeholder="900"
          />
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={form.advanced_mode === "true"}
              onChange={(e) => updateField("advanced_mode", e.target.checked ? "true" : "false")}
            />
            Show Portfolio in the sidebar
          </label>
        </div>
      </details>

      {dirty && (
        <StickyFooter>
          <Button onClick={handleSave} disabled={saving}>
            {saving ? "Saving…" : "Save settings"}
          </Button>
        </StickyFooter>
      )}
    </div>
  );
}

/**
 * Start/stop the app-managed hsd node and show its live status. hsd is launched
 * with the configured data directory; `dirty` warns that an unsaved directory
 * change won't apply until settings are saved.
 */
function NodeControl({ dirty }: { dirty: boolean }) {
  const { data: status } = useNodeStatus();
  const start = useStartHsd();
  const stop = useStopHsd();
  const showToast = useUiStore((s) => s.showToast);

  const connected = status?.connected ?? false;
  const processAlive = status?.process_alive ?? false;
  // A node that doesn't report progress is treated as synced (covers nodes that
  // omit verificationProgress); otherwise it's synced at ~100%.
  const progress = status?.verification_progress ?? null;
  const synced = progress == null || progress >= 0.9999;
  const pct = progress == null ? 100 : Math.floor(progress * 1000) / 10; // 1 decimal
  // Connected (RPC answers) → green; spawned but RPC not up yet → amber; else grey.
  const dotClass = connected ? "bg-green-500" : processAlive ? "bg-amber-500" : "bg-gray-300";
  const label = connected
    ? synced
      ? `Connected · block ${status?.height ?? "?"}${processAlive ? "" : " (external node)"}`
      : `Syncing · ${pct}%`
    : processAlive
      ? "Starting…"
      : "Stopped";

  const onStart = async () => {
    try {
      const res = await start.mutateAsync();
      if (res?.connected) {
        showToast("hsd connected", "success");
      } else {
        showToast("hsd is starting… status will update when its RPC responds.", "info");
      }
    } catch (e) {
      showToast(`Failed to start hsd: ${e}`, "error");
    }
  };
  const onStop = async () => {
    try {
      await stop.mutateAsync();
      showToast("hsd stopped", "success");
    } catch (e) {
      showToast(`Failed to stop hsd: ${e}`, "error");
    }
  };

  return (
    <div className="rounded border border-gray-200 bg-gray-50 p-3 space-y-2">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 text-sm">
          <span className={`inline-block w-2 h-2 rounded-full ${dotClass}`} />
          <span className="font-medium">{label}</span>
        </div>
        {processAlive ? (
          <Button size="sm" variant="secondary" onClick={onStop} disabled={stop.isPending}>
            {stop.isPending ? "Stopping…" : "Stop hsd"}
          </Button>
        ) : (
          <Button
            size="sm"
            onClick={onStart}
            disabled={start.isPending || connected || !status?.binary_found}
          >
            {start.isPending ? "Starting…" : "Start hsd"}
          </Button>
        )}
      </div>
      {connected && !synced && (
        <div className="space-y-1">
          <div className="h-1.5 w-full rounded-full bg-gray-200 overflow-hidden">
            <div
              className="h-full bg-blue-500 transition-all"
              style={{ width: `${pct}%` }}
            />
          </div>
          <div className="text-xs text-gray-500">
            Syncing the chain — {pct}% · block {status?.height ?? "?"}. Spendable
            balance and sending become available once it finishes.
          </div>
        </div>
      )}
      <div className="text-xs text-gray-500 space-y-0.5">
        <div>
          Data dir: <code>{status?.data_dir ?? "…"}</code>
        </div>
        <div>
          {status?.binary_found ? (
            <>
              hsd {status.version} · {status.network}
            </>
          ) : (
            <span className="text-red-600">
              hsd binary not found — install it (<code>npm i -g hsd</code>).
            </span>
          )}
        </div>
        {dirty && (
          <div className="text-amber-600">
            Save settings to apply a new data directory before starting.
          </div>
        )}
      </div>
    </div>
  );
}
