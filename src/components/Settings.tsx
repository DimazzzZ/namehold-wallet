import { useState, useEffect } from "react";
import { useSettingsStore } from "../stores/settings";
import { useNodeStatus, useStartHsd, useStopHsd, useResyncHsd } from "../queries/node";
import { useActiveProfile, useExportBidCommitments } from "../queries/wallet";
import { open, save } from "../lib/dialog";
import { isTauri } from "../lib/runtime";
import { invoke } from "../lib/invoke";
import {
  checkNotificationPermission,
  requestNotificationPermission,
  type PermissionStatus,
} from "../lib/notifications";
import { Input } from "./ui/Input";
import { Button } from "./ui/Button";
import { StickyFooter } from "./ui/StickyFooter";
import { useUiStore } from "../stores/ui";
import { UpdatesSettings } from "./UpdatesSettings";

/**
 * Validate the explorer base URL field (Task 11 / S1). Empty is allowed —
 * it falls back to the backend's own default (`DEFAULT_EXPLORER_URL`) — but
 * anything non-empty must carry an `http(s)://` scheme, since the backend
 * builds requests as `${url}/api/...` and a bare host/path would silently
 * fail every explorer call. Returns an error message, or `null` when valid.
 */
export function validateExplorerUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  if (!/^https?:\/\/[^\s]+$/i.test(trimmed)) {
    return "Explorer URL must start with http:// or https://";
  }
  return null;
}

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
  const { data: profile } = useActiveProfile();
  const exportBids = useExportBidCommitments();

  const [form, setForm] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    if (settings) {
      setForm({
        node_rpc_url: settings.node_rpc_url,
        node_rpc_api_key: settings.node_rpc_api_key,
        hsd_prefix: settings.hsd_prefix,
        hsd_path: settings.hsd_path,
        autostart_hsd: settings.autostart_hsd,
        background_sync_enabled: settings.background_sync_enabled,
        node_mode: settings.node_mode,
        explorer_api_url: settings.explorer_api_url,
        explorer_fallback_url: settings.explorer_fallback_url,
        chain_source: settings.chain_source,
        address_gap_limit: settings.address_gap_limit,
        signer_session_timeout_seconds: settings.signer_session_timeout_seconds,
        advanced_mode: settings.advanced_mode,
        deadline_notify_enabled: settings.deadline_notify_enabled,
        deadline_notify_reveal_lead_blocks: settings.deadline_notify_reveal_lead_blocks,
        deadline_notify_renewal_lead_days: settings.deadline_notify_renewal_lead_days,
        watchlist_notify_enabled: settings.watchlist_notify_enabled,
        watchlist_notify_bidding_soon_lead_blocks:
          settings.watchlist_notify_bidding_soon_lead_blocks,
        watchlist_notify_highest_bid_threshold_hns:
          settings.watchlist_notify_highest_bid_threshold_hns,
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

  // Explorer base URL is used as `${url}/api/...` by the backend
  // (HnsFansClient / explorer_client_from_settings — Task 11 / S1), so it
  // must carry a scheme; empty is allowed (falls back to the backend
  // default). Returns null when valid.
  const explorerUrlError = validateExplorerUrl(form.explorer_api_url ?? "");

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
    if (explorerUrlError) {
      showToast(explorerUrlError, "error");
      return;
    }
    // Normalize the trailing slash the same way the backend does
    // (`HnsFansClient::new` trims it) so the value shown here always matches
    // what's actually used — a stray slash would build `//api/...` URLs.
    const normalized: Record<string, string> = {
      ...form,
      explorer_api_url: (form.explorer_api_url ?? "").trim().replace(/\/+$/, ""),
    };
    // Secret fields (currently `node_rpc_api_key`) are write-only: the backend
    // never returns their value, so the form always starts empty even when one
    // is stored. Saving that empty value would clobber the stored secret.
    // Skip the field on save when it's still empty AND the backend reported a
    // stored value via the `__has_<key>` marker.
    const hasStoredApiKey =
      (settings as unknown as Record<string, string>)["__has_node_rpc_api_key"] === "true";
    if (hasStoredApiKey && (normalized.node_rpc_api_key ?? "") === "") {
      delete normalized.node_rpc_api_key;
    }
    setSaving(true);
    try {
      await saveAll(normalized);
      setForm(normalized);
      setDirty(false);
      showToast("Settings saved", "success");
    } catch (e) {
      showToast(`Failed to save: ${e}`, "error");
    } finally {
      setSaving(false);
    }
  };

  // Export every bid commitment (value, blind, nonce) for the active wallet
  // as a JSON backup file. This is the ONLY off-chain copy of a bid's true
  // value/nonce — losing it (without this backup) makes the lockup
  // unrecoverable unless the user still remembers their bid amount.
  const handleExportBidBackup = async () => {
    if (!profile) {
      showToast("No active wallet profile", "error");
      return;
    }
    try {
      const json = await exportBids.mutateAsync(profile.id);
      const path = await save({
        filters: [{ name: "JSON", extensions: ["json"] }],
        defaultPath: `${profile.label || "wallet"}-bid-backup.json`,
      });
      if (!path) return;
      if (isTauri()) {
        const { writeTextFile } = await import("@tauri-apps/plugin-fs");
        await writeTextFile(path, json);
      }
      showToast("Bid backup exported", "success");
    } catch (e) {
      showToast(`Bid backup export failed: ${e}`, "error");
    }
  };

  return (
    <div className="space-y-6 max-w-xl pb-16">
      <h2 className="text-xl font-bold">Settings</h2>

      {/* Connections: reads (explorer) + sending (node) in one place. */}
      <div className="bg-white rounded p-4 border border-gray-200 space-y-4">
        <h3 className="text-sm font-semibold text-gray-700">Connections</h3>

        <div className="space-y-2">
          <Input
            label="Explorer base URL (reads)"
            value={form.explorer_api_url ?? ""}
            onChange={(e) => updateField("explorer_api_url", e.target.value)}
            placeholder="https://e.hnsfans.com"
            data-testid="explorer-url-input"
          />
          {explorerUrlError ? (
            <div className="text-xs text-red-600" data-testid="explorer-url-error">
              {explorerUrlError}
            </div>
          ) : (
            <div className="text-xs text-gray-500">
              Balance and names are read from this explorer when the node is not
              synced. When the node is connected and fully synced, reads come from
              the local node cache instead. Takes effect on the next Sync/read.
            </div>
          )}
          <Input
            label="Explorer fallback URL (optional)"
            value={form.explorer_fallback_url ?? ""}
            onChange={(e) => updateField("explorer_fallback_url", e.target.value)}
            placeholder="https://shakeshift.com"
            data-testid="explorer-fallback-url-input"
          />
          <div className="text-xs text-gray-500">
            Used when the primary explorer is unreachable. Leave empty to disable failover.
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
            placeholder={
              (settings as unknown as Record<string, string>)["__has_node_rpc_api_key"] === "true"
                ? "•••••• (stored — leave blank to keep)"
                : "(optional)"
            }
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

          <Input
            label="hsd binary path (optional)"
            value={form.hsd_path ?? ""}
            onChange={(e) => updateField("hsd_path", e.target.value)}
            placeholder="(auto-detect: Homebrew / npm / nvm / PATH)"
          />
          <div className="text-xs text-gray-500">
            Leave empty to auto-detect. Set this if the app can't find your hsd
            install (e.g. <code>$(which hsd)</code>). Save settings to apply.
          </div>

          {/* Node mode dropdown: only visible when using a local or remote node.
              When chain_source is "explorer", the node mode is irrelevant. */}
          {(form.chain_source ?? "local_node") !== "explorer" && (
            <div className="space-y-2 pt-2 border-t border-gray-100">
              <label className="text-sm font-medium">Node mode</label>
              <select
                value={form.node_mode ?? "full"}
                onChange={(e) => updateField("node_mode", e.target.value)}
                className="w-full border border-gray-300 rounded px-2 py-1.5 text-sm"
                data-testid="node-mode-select"
              >
                <option value="full">Full node (requires ~15GB, indexes all addresses)</option>
                <option value="spv">Lightweight SPV (faster sync, uses explorer for data)</option>
              </select>
              <div className="text-xs text-gray-500">
                SPV mode downloads only block headers. Faster initial sync, less disk space.
                Balance and name data come from the explorer. Requires hsd restart to apply.
                If switching causes issues, use "Re-sync node data" below.
              </div>
            </div>
          )}

          <label className="flex items-center gap-2 text-sm pt-2">
            <input
              type="checkbox"
              checked={form.autostart_hsd === "true"}
              onChange={(e) =>
                updateField("autostart_hsd", e.target.checked ? "true" : "false")
              }
              data-testid="autostart-hsd-checkbox"
            />
            Autostart HSD when the app launches
          </label>
          <div className="text-xs text-gray-500">
            Starts hsd against your data dir on launch. If a node is already
            running, Namehold adopts it instead of starting a duplicate. Change
            takes effect on the next launch.
          </div>

          <label className="flex items-center gap-2 text-sm pt-2">
            <input
              type="checkbox"
              checked={form.background_sync_enabled === "1"}
              onChange={async (e) => {
                const enabled = e.target.checked;
                // Update local form state immediately for responsive UI.
                updateField("background_sync_enabled", enabled ? "1" : "0");
                try {
                  // The specialized command also spawns/stops the daemon,
                  // and persists the setting itself. Applied immediately (no
                  // Save button) so users see the daemon start/stop right away.
                  await invoke("set_background_sync_enabled", { enabled });
                  showToast(
                    enabled
                      ? "Background sync enabled — daemon started"
                      : "Background sync disabled — daemon stopped",
                    "success",
                  );
                } catch (err) {
                  // Revert form state if the command failed.
                  updateField("background_sync_enabled", enabled ? "0" : "1");
                  showToast(`Failed to toggle background sync: ${err}`, "error");
                }
              }}
              data-testid="background-sync-checkbox"
            />
            Sync in background (keep wallet up to date when app is closed)
          </label>
          <div className="text-xs text-gray-500">
            Runs a lightweight sync process every 60 seconds. When enabled, the
            local hsd node keeps running after you close the app so the daemon
            can query it. The next app launch adopts the running node — no
            duplicate is spawned.
          </div>

          <NodeControl dirty={dirty} hsdPathConfigured={!!settings.hsd_path?.trim()} />
        </div>
      </div>

      {/* Backup: bid commitments (value/blind/nonce) are the only off-chain
          copy — losing the DB row without a backup can make an in-flight
          lockup unrecoverable. */}
      <div className="bg-white rounded p-4 border border-gray-200 space-y-3">
        <h3 className="text-sm font-semibold text-gray-700">Backup</h3>
        <div className="text-xs text-gray-500">
          Your bid commitments (amount, blind, nonce) for open auctions live
          only in this wallet&apos;s local database — the blockchain only ever
          sees the blind. Export a backup and store it alongside your seed
          phrase, in case this device is lost.
        </div>
        <Button
          size="sm"
          variant="secondary"
          onClick={handleExportBidBackup}
          disabled={exportBids.isPending || !profile}
          data-testid="export-bid-backup"
        >
          {exportBids.isPending ? "Exporting…" : "Export bid backup"}
        </Button>
      </div>

      {/* Deadline notifications (I1): opt-in OS alerts before a reveal
          window or renewal deadline is missed. */}
      <div className="bg-white rounded p-4 border border-gray-200 space-y-3">
        <h3 className="text-sm font-semibold text-gray-700">Notifications</h3>
        <div className="text-xs text-gray-500">
          Get an OS notification before a bid&apos;s reveal window closes
          (miss it and the lockup is forfeit) or a name&apos;s renewal is due.
          Checked on app start and every ~10 minutes.
        </div>
        <NotificationSettings form={form} updateField={updateField} />
      </div>

      {/* Watchlist notifications: opt-in OS alerts about WATCHED (non-owned)
          names. Emitted by the background daemon (namehold-syncd), so they
          fire even when the app is closed (requires background sync on). */}
      <div className="bg-white rounded p-4 border border-gray-200 space-y-3">
        <h3 className="text-sm font-semibold text-gray-700">Watchlist notifications</h3>
        <div className="text-xs text-gray-500">
          Get an OS notification when a name on your watchlist enters bidding,
          becomes available again, is about to open for bidding, or when its
          highest bid crosses a threshold. Handled by the background sync
          daemon, so alerts fire even when the app is closed.
        </div>
        <WatchlistNotificationSettings form={form} updateField={updateField} />
      </div>

      {/* Updates: shows the running version and drives the check-for-updates
          flow (shared state with the global update banner). */}
      <div className="bg-white rounded p-4 border border-gray-200 space-y-3">
        <h3 className="text-sm font-semibold text-gray-700">Updates</h3>
        <UpdatesSettings />
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
          <Button onClick={handleSave} disabled={saving || !!explorerUrlError}>
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
function NodeControl({ dirty, hsdPathConfigured }: { dirty: boolean; hsdPathConfigured: boolean }) {
  const { data: status } = useNodeStatus();
  const start = useStartHsd();
  const stop = useStopHsd();
  const resync = useResyncHsd();
  const showToast = useUiStore((s) => s.showToast);

  const connected = status?.connected ?? false;
  const processAlive = status?.process_alive ?? false;
  // "Synced" = chain tip reached (applied blocks caught up to best header).
  // verificationProgress can plateau just under 1.0 (e.g. ~0.9997 on regtest), so
  // it's only a fallback when the node doesn't report headers.
  const height = status?.height ?? null;
  const headers = status?.headers ?? null;
  const progress = status?.verification_progress ?? null;
  // When verification_progress is available it is the most reliable signal —
  // a node can report height == headers while still only ~8% verified if it
  // is far behind the real chain tip. Always gate on progress when present.
  const synced =
    progress != null
      ? progress >= 0.9999
      : headers != null && headers > 0
        ? height != null && height >= headers
        : true;
  const pct =
    progress != null
      ? Math.floor(progress * 1000) / 10
      : headers != null && headers > 0 && height != null
        ? Math.min(100, Math.floor((height / headers) * 1000) / 10)
        : 100; // 1 decimal
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
  const onResync = async () => {
    if (
      !window.confirm(
        "Re-sync node data?\n\nYour current chain will be moved to a timestamped " +
          "backup folder in the data directory, and hsd will re-sync from scratch " +
          "with the indexes the wallet needs. This can take a while. Your wallet " +
          "keys are NOT affected.",
      )
    )
      return;
    try {
      await resync.mutateAsync();
      showToast("Re-syncing: old chain backed up; hsd is downloading again.", "info");
    } catch (e) {
      showToast(`Failed to re-sync: ${e}`, "error");
    }
  };

  return (
    <div className="rounded border border-gray-200 bg-gray-50 p-3 space-y-2">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 text-sm">
          <span className={`inline-block w-2 h-2 rounded-full ${dotClass}`} />
          <span className="font-medium">{label}</span>
        </div>
        {processAlive || connected ? (
          <Button size="sm" variant="secondary" onClick={onStop} disabled={stop.isPending}>
            {stop.isPending ? "Stopping…" : "Stop hsd"}
          </Button>
        ) : (
          <Button
            size="sm"
            onClick={onStart}
            disabled={start.isPending || connected || (!status?.binary_found && !hsdPathConfigured)}
          >
            {start.isPending ? "Starting…" : "Start hsd"}
          </Button>
        )}
      </div>
      {connected && (
        <div className="space-y-1" data-testid="node-sync-progress">
          <div className="h-1.5 w-full rounded-full bg-gray-200 overflow-hidden">
            <div
              className={`h-full transition-all ${synced ? "bg-green-500" : "bg-blue-500"}`}
              style={{ width: `${synced ? 100 : pct}%` }}
            />
          </div>
          <div className="text-xs text-gray-500">
            {synced ? (
              <>
                Synced — 100% · block {status?.height ?? "?"}
                {headers ? ` / ${headers}` : ""}.
              </>
            ) : (
              <>
                Syncing the chain — {pct}% · block {status?.height ?? "?"}
                {headers != null && height != null && headers > height
                  ? ` / ${headers}`
                  : ""}
                . Spendable balance and sending become available once it finishes.
              </>
            )}
          </div>
        </div>
      )}
        <div className="text-xs text-gray-500 space-y-0.5">
        <div>
          Read source: <span className="font-medium">{status?.read_source === "local" ? "Local node cache" : "Explorer"}</span>
        </div>
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
      {/* Why the last start failed — shown when the RPC isn't answering, so a
          failed start never looks like a silent "Starting…". */}
      {!connected && status?.last_error && (
        <div className="space-y-2">
          <pre
            className="text-xs text-red-700 bg-red-50 border border-red-200 rounded p-2 whitespace-pre-wrap break-words"
            data-testid="node-last-error"
          >
            {status.last_error}
          </pre>
          {status.index_mismatch && (
            <Button
              size="sm"
              variant="primary"
              onClick={onResync}
              disabled={resync.isPending}
              data-testid="node-resync"
            >
              {resync.isPending ? "Re-syncing…" : "Re-sync node data"}
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * Enable/disable toggle + lead-time inputs for the deadline notification
 * scanner (I1). The toggle drives an immediate OS permission request (must
 * happen from this user gesture — macOS silently denies requests made
 * without one) independent of the outer form's Save button; the enabled
 * flag and lead times themselves are plain form fields saved the same way
 * as every other setting.
 */
function NotificationSettings({
  form,
  updateField,
}: {
  form: Record<string, string>;
  updateField: (key: string, value: string) => void;
}) {
  const [permission, setPermission] = useState<PermissionStatus | null>(null);
  const [requesting, setRequesting] = useState(false);
  const enabled = form.deadline_notify_enabled === "true";

  useEffect(() => {
    checkNotificationPermission().then(setPermission);
  }, []);

  const onToggle = async (checked: boolean) => {
    updateField("deadline_notify_enabled", checked ? "true" : "false");
    if (!checked) return;
    setRequesting(true);
    try {
      const status = await requestNotificationPermission();
      setPermission(status);
    } finally {
      setRequesting(false);
    }
  };

  return (
    <div className="space-y-3">
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => onToggle(e.target.checked)}
          data-testid="deadline-notify-toggle"
        />
        Enable deadline notifications
      </label>

      {enabled && (
        <>
          {permission === "denied" && (
            <div
              className="text-xs text-amber-600 bg-amber-50 border border-amber-200 rounded p-2"
              data-testid="notification-permission-denied"
            >
              OS notifications are blocked for this app. Enable them in your
              system notification settings — deadlines will still show
              in-app, but you won&apos;t get an alert when the app isn&apos;t
              open.
            </div>
          )}
          {permission === "unsupported" && (
            <div className="text-xs text-gray-500">
              OS notifications aren&apos;t available outside the desktop app.
            </div>
          )}
          {requesting && (
            <div className="text-xs text-gray-500">Requesting permission…</div>
          )}

          <div className="grid grid-cols-2 gap-3">
            <Input
              label="Reveal window lead time (blocks)"
              value={form.deadline_notify_reveal_lead_blocks ?? ""}
              onChange={(e) => updateField("deadline_notify_reveal_lead_blocks", e.target.value)}
              placeholder="144"
            />
            <Input
              label="Renewal lead time (days)"
              value={form.deadline_notify_renewal_lead_days ?? ""}
              onChange={(e) => updateField("deadline_notify_renewal_lead_days", e.target.value)}
              placeholder="30"
            />
          </div>
        </>
      )}
    </div>
  );
}

function WatchlistNotificationSettings({
  form,
  updateField,
}: {
  form: Record<string, string>;
  updateField: (key: string, value: string) => void;
}) {
  const [permission, setPermission] = useState<PermissionStatus | null>(null);
  const [requesting, setRequesting] = useState(false);
  const enabled = form.watchlist_notify_enabled === "true";

  useEffect(() => {
    checkNotificationPermission().then(setPermission);
  }, []);

  const onToggle = async (checked: boolean) => {
    updateField("watchlist_notify_enabled", checked ? "true" : "false");
    if (!checked) return;
    setRequesting(true);
    try {
      const status = await requestNotificationPermission();
      setPermission(status);
    } finally {
      setRequesting(false);
    }
  };

  return (
    <div className="space-y-3">
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => onToggle(e.target.checked)}
          data-testid="watchlist-notify-toggle"
        />
        Enable watchlist notifications
      </label>

      {enabled && (
        <>
          {permission === "denied" && (
            <div
              className="text-xs text-amber-600 bg-amber-50 border border-amber-200 rounded p-2"
              data-testid="watchlist-notification-permission-denied"
            >
              OS notifications are blocked for this app. Enable them in your
              system notification settings — otherwise watchlist alerts
              won&apos;t reach you.
            </div>
          )}
          {permission === "unsupported" && (
            <div className="text-xs text-gray-500">
              OS notifications aren&apos;t available outside the desktop app.
            </div>
          )}
          {requesting && (
            <div className="text-xs text-gray-500">Requesting permission…</div>
          )}

          <div className="grid grid-cols-2 gap-3">
            <Input
              label="Bidding-soon lead time (blocks)"
              type="number"
              value={form.watchlist_notify_bidding_soon_lead_blocks ?? ""}
              onChange={(e) =>
                updateField("watchlist_notify_bidding_soon_lead_blocks", e.target.value)
              }
              placeholder="144"
              data-testid="watchlist-notify-bidding-lead-input"
            />
            <Input
              label="Highest-bid alert threshold (HNS, blank = off)"
              type="number"
              step="0.01"
              value={form.watchlist_notify_highest_bid_threshold_hns ?? ""}
              onChange={(e) =>
                updateField("watchlist_notify_highest_bid_threshold_hns", e.target.value)
              }
              placeholder="e.g. 100"
              data-testid="watchlist-notify-highbid-input"
            />
          </div>
        </>
      )}
    </div>
  );
}
