import { useState, useEffect } from "react";
import { useSettingsStore } from "../stores/settings";
import { Input } from "./ui/Input";
import { Select } from "./ui/Select";
import { Button } from "./ui/Button";
import { StickyFooter } from "./ui/StickyFooter";
import { useUiStore } from "../stores/ui";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "../lib/invoke";
import { mapError } from "../lib/errors";

export function Settings() {
  const { settings, loaded, saveAll, passphrase, setPassphrase, clearPassphrase } = useSettingsStore();
  const showToast = useUiStore((s) => s.showToast);

  const [form, setForm] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [testingConnection, setTestingConnection] = useState(false);
  const [connectionResult, setConnectionResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [showAdvancedExternal, setShowAdvancedExternal] = useState(false);
  // Progressive disclosure: most users never need node URLs, API keys, data
  // directory, ports, or connection-mode internals. Keep these collapsed by
  // default so the screen reads as a short, approachable set of essentials.
  const [showAdvanced, setShowAdvanced] = useState(false);

  useEffect(() => {
    if (settings) {
      setForm({
        hsd_wallet_api_url: settings.hsd_wallet_api_url,
        hsd_node_api_url: settings.hsd_node_api_url,
        hsd_api_key: settings.hsd_api_key,
        hsd_wallet_id: settings.hsd_wallet_id,
        hsd_network: settings.hsd_network,
        hsd_prefix: settings.hsd_prefix,
        write_mode: settings.write_mode,
        connection_mode: settings.connection_mode || "local_managed_hsd",
        external_read_provider: settings.external_read_provider || "none",
        external_read_api_url: settings.external_read_api_url || "",
        external_read_watch_addresses: settings.external_read_watch_addresses || "[]",
        external_read_watch_names: settings.external_read_watch_names || "[]",
        remote_hsd_label: settings.remote_hsd_label || "",
        trusted_remote_hsd: settings.trusted_remote_hsd || "false",
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

  const handleBrowsePrefix = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (selected) {
      updateField("hsd_prefix", selected as string);
    }
  };

  const handleTestConnection = async () => {
    setTestingConnection(true);
    setConnectionResult(null);
    try {
      await saveAll(form);
      const result = await invoke<{ connected: boolean; info?: unknown; error?: string }>("check_connection");
      if (result.connected) {
        setConnectionResult({ ok: true, message: "Connected to hsd successfully" });
      } else {
        setConnectionResult({ ok: false, message: result.error || "Cannot connect to hsd" });
      }
    } catch (e) {
      setConnectionResult({ ok: false, message: mapError(e) });
    } finally {
      setTestingConnection(false);
    }
  };

  const isLocalhost =
    (form.hsd_wallet_api_url || "").includes("127.0.0.1") ||
    (form.hsd_wallet_api_url || "").includes("localhost");

  const connectionMode = form.connection_mode || "local_managed_hsd";
  const externalProvider = form.external_read_provider || "none";
  const trustedRemote = form.trusted_remote_hsd === "true";

  const parseJsonLines = (raw: string): string[] => {
    try {
      const parsed = JSON.parse(raw || "[]");
      return Array.isArray(parsed) ? parsed.map((x) => String(x)) : [];
    } catch {
      return [];
    }
  };

  const linesToJson = (text: string): string =>
    JSON.stringify(
      text
        .split(/[\n,]/)
        .map((s) => s.trim())
        .filter(Boolean),
    );

  const watchAddressesText = parseJsonLines(form.external_read_watch_addresses || "").join("\n");
  const watchNamesText = parseJsonLines(form.external_read_watch_names || "").join("\n");

  return (
    <div className="space-y-6 max-w-xl pb-16">
      <h2 className="text-xl font-bold">Settings</h2>

      {/* Essentials — what most users actually touch. */}
      <div className="bg-white rounded p-4 border border-gray-200 space-y-4">
        <h3 className="text-sm font-semibold text-gray-700">Wallet</h3>
        <Input
          label="Wallet ID"
          value={form.hsd_wallet_id || ""}
          onChange={(e) => updateField("hsd_wallet_id", e.target.value)}
        />
        <Select
          label="Network"
          options={[
            { value: "mainnet", label: "Mainnet" },
            { value: "testnet", label: "Testnet" },
            { value: "regtest", label: "Regtest" },
          ]}
          value={form.hsd_network || "mainnet"}
          onChange={(e) => updateField("hsd_network", e.target.value)}
        />
      </div>

      {/* Security essentials — write mode + session passphrase. */}
      <div className="bg-white rounded p-4 border border-gray-200 space-y-4">
        <h3 className="text-sm font-semibold text-gray-700">Security</h3>
        <div>
          <div className="text-sm font-medium mb-1">Write Mode</div>
          <div className="text-xs text-gray-500 mb-3">
            Write mode enables actions that modify your wallet: send HNS, transfer TLDs, renew names, update records.
            Disabled by default for safety.
          </div>
          {form.write_mode === "true" ? (
            <div className="space-y-2">
              <div className="bg-green-50 border border-green-200 rounded p-2 text-xs text-green-700">
                Write mode is enabled. Write actions are available.
              </div>
              <Button
                variant="danger"
                size="sm"
                onClick={() => updateField("write_mode", "false")}
              >
                Disable Write Mode
              </Button>
            </div>
          ) : (
            <div className="space-y-2">
              <div className="bg-gray-50 border border-gray-200 rounded p-2 text-xs text-gray-600">
                Write mode is disabled. Only read operations are available.
              </div>
              <Button
                variant="primary"
                size="sm"
                onClick={() => {
                  if (confirm("Enable write mode? This allows sending HNS, transferring TLDs, and other wallet operations. Use with caution.")) {
                    updateField("write_mode", "true");
                  }
                }}
              >
                Enable Write Mode
              </Button>
            </div>
          )}
        </div>
        <div>
          <div className="text-sm font-medium mb-1">Wallet Passphrase (memory only)</div>
          <div className="text-xs text-gray-500 mb-2">
            Stored in memory only. Lost on app restart. Used for write operations (send, transfer, renew, finalize).
          </div>
          <div className="flex gap-2">
            <Input
              type="password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
              placeholder="Enter wallet passphrase"
              className="flex-1"
            />
            {passphrase && (
              <Button size="sm" variant="ghost" onClick={clearPassphrase}>
                Clear
              </Button>
            )}
          </div>
        </div>
      </div>

      {/* Advanced — node connection internals, hidden by default. */}
      <div className="bg-white rounded border border-gray-200">
        <button
          type="button"
          className="w-full flex items-center justify-between px-4 py-3 text-sm font-semibold text-gray-700"
          onClick={() => setShowAdvanced((v) => !v)}
        >
          <span>Advanced (connection & node)</span>
          <span className="text-gray-400 text-xs">{showAdvanced ? "Hide" : "Show"}</span>
        </button>

        {showAdvanced && (
          <div className="px-4 pb-4 space-y-6">
            <div className="space-y-4">
              <h3 className="text-sm font-semibold text-gray-700">Handshake Node</h3>
              <Input
                label="Wallet API URL"
                value={form.hsd_wallet_api_url || ""}
                onChange={(e) => updateField("hsd_wallet_api_url", e.target.value)}
              />
              {!isLocalhost && (
                <div className="bg-red-50 border border-red-200 rounded p-2 text-xs text-red-700">
                  Warning: Non-localhost URL. Only use local connections for security.
                </div>
              )}
              <Input
                label="Node API URL"
                value={form.hsd_node_api_url || ""}
                onChange={(e) => updateField("hsd_node_api_url", e.target.value)}
              />
              <Input
                label="API Key"
                type="password"
                value={form.hsd_api_key || ""}
                onChange={(e) => updateField("hsd_api_key", e.target.value)}
                placeholder="Leave empty if no auth"
              />
              <div className="flex gap-2 items-end">
                <div className="flex-1">
                  <Input
                    label="Data Directory (hsd prefix)"
                    value={form.hsd_prefix || ""}
                    onChange={(e) => updateField("hsd_prefix", e.target.value)}
                    placeholder="~/.hsd (default) or /Volumes/WD/hsd-data"
                  />
                </div>
                <Button size="sm" onClick={handleBrowsePrefix}>
                  Browse
                </Button>
              </div>
              <div className="text-xs text-gray-500">
                Path where hsd stores blockchain and wallet data. Use an external drive for large blockchain data.
              </div>
              <div className="flex items-center gap-3">
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={handleTestConnection}
                  disabled={testingConnection}
                >
                  {testingConnection ? "Testing..." : "Test Connection"}
                </Button>
                {connectionResult && (
                  <span className={`text-sm ${connectionResult.ok ? "text-green-600" : "text-red-600"}`}>
                    {connectionResult.message}
                  </span>
                )}
              </div>
            </div>

            <div className="space-y-4">
              <h3 className="text-sm font-semibold text-gray-700">Connection Mode</h3>
              <div className="text-xs text-gray-500">
                Choose how the wallet reads chain data. External read-only mode requires no local node
                but cannot sign or broadcast transactions.
              </div>
              <Select
                label="Mode"
                options={[
                  { value: "local_managed_hsd", label: "Local managed hsd (full read + write)" },
                  { value: "remote_hsd", label: "Remote hsd (requires trust to write)" },
                  { value: "auto_fallback", label: "Auto fallback (prefer hsd, fall back to external)" },
                  { value: "external_read_only", label: "External read-only (no writes)" },
                ]}
                value={connectionMode}
                onChange={(e) => updateField("connection_mode", e.target.value)}
              />

              {connectionMode === "remote_hsd" && (
                <div className="space-y-3 border-l-2 border-blue-200 pl-3">
                  <Input
                    label="Remote hsd Label"
                    value={form.remote_hsd_label || ""}
                    onChange={(e) => updateField("remote_hsd_label", e.target.value)}
                    placeholder="e.g. home-server"
                  />
                  <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
                    Writes to a remote hsd send your transactions through a node you do not manage.
                    Only trust nodes you fully control.
                  </div>
                  <label className="flex items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      checked={trustedRemote}
                      onChange={(e) =>
                        updateField("trusted_remote_hsd", e.target.checked ? "true" : "false")
                      }
                    />
                    I trust this remote hsd for write operations
                  </label>
                </div>
              )}

              {(connectionMode === "external_read_only" || connectionMode === "auto_fallback") && (
                <div className="space-y-3 border-l-2 border-blue-200 pl-3">
                  <Select
                    label="External Read Provider"
                    options={[
                      { value: "none", label: "None" },
                      { value: "hnsfans", label: "hnsfans explorer" },
                    ]}
                    value={externalProvider}
                    onChange={(e) => updateField("external_read_provider", e.target.value)}
                  />
                  {externalProvider !== "none" && (
                    <>
                      <div className="bg-blue-50 border border-blue-200 rounded p-2 text-xs text-blue-800">
                        This wallet automatically reads its known addresses (from past
                        syncs) and the names in your inventory. You usually don't need
                        to configure anything else. Open Advanced only to override the
                        provider URL or watch extra addresses/names.
                      </div>
                      <button
                        type="button"
                        className="text-xs text-blue-600 hover:underline self-start"
                        onClick={() => setShowAdvancedExternal((v) => !v)}
                      >
                        {showAdvancedExternal ? "Hide advanced" : "Advanced settings"}
                      </button>
                      {showAdvancedExternal && (
                        <div className="space-y-3 border-l-2 border-gray-200 pl-3">
                          <Input
                            label="External Read API URL"
                            value={form.external_read_api_url || ""}
                            onChange={(e) => updateField("external_read_api_url", e.target.value)}
                            placeholder="https://hnsfans.com (default)"
                          />
                          <div className="flex flex-col gap-1">
                            <label className="text-sm font-medium text-gray-700">
                              Extra Watch Addresses
                            </label>
                            <textarea
                              className="border border-gray-300 rounded px-3 py-2 text-sm h-20 resize-none font-mono focus:outline-none focus:ring-2 focus:ring-blue-500"
                              value={watchAddressesText}
                              onChange={(e) =>
                                updateField("external_read_watch_addresses", linesToJson(e.target.value))
                              }
                              placeholder="hs1q... (one per line; leave empty to use cached wallet addresses)"
                            />
                          </div>
                          <div className="flex flex-col gap-1">
                            <label className="text-sm font-medium text-gray-700">Extra Watch Names</label>
                            <textarea
                              className="border border-gray-300 rounded px-3 py-2 text-sm h-20 resize-none font-mono focus:outline-none focus:ring-2 focus:ring-blue-500"
                              value={watchNamesText}
                              onChange={(e) =>
                                updateField("external_read_watch_names", linesToJson(e.target.value))
                              }
                              placeholder="example (one per line, no .; leave empty to use your inventory)"
                            />
                          </div>
                        </div>
                      )}
                    </>
                  )}
                  <div className="bg-gray-50 border border-gray-200 rounded p-2 text-xs text-gray-600">
                    External read-only mode displays balances, names, and transactions from a public
                    explorer. Sending HNS and on-chain name operations are disabled.
                  </div>
                </div>
              )}
            </div>

            <div className="space-y-2">
              <h3 className="text-sm font-semibold text-gray-700">Default Ports</h3>
              <table className="text-xs text-gray-600">
                <tbody>
                  <tr><td className="pr-4 py-0.5">Mainnet Wallet:</td><td>12039</td></tr>
                  <tr><td className="pr-4 py-0.5">Mainnet Node:</td><td>12037</td></tr>
                  <tr><td className="pr-4 py-0.5">Testnet Wallet:</td><td>13039</td></tr>
                  <tr><td className="pr-4 py-0.5">Testnet Node:</td><td>13037</td></tr>
                  <tr><td className="pr-4 py-0.5">Regtest Wallet:</td><td>14039</td></tr>
                  <tr><td className="pr-4 py-0.5">Regtest Node:</td><td>14037</td></tr>
                </tbody>
              </table>
            </div>
          </div>
        )}
      </div>

      <StickyFooter>
        <span className="text-sm text-gray-500">
          {dirty ? "You have unsaved changes" : "All changes saved"}
        </span>
        <Button variant="primary" onClick={handleSave} disabled={saving || !dirty}>
          {saving ? "Saving..." : "Save Settings"}
        </Button>
      </StickyFooter>
    </div>
  );
}
