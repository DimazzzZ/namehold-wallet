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
  const [namebaseCookie, setNamebaseCookie] = useState("");
  const [namebaseConnected, setNamebaseConnected] = useState(false);
  const [connectingNamebase, setConnectingNamebase] = useState(false);

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
      });
      // Load Namebase connection status
      invoke<{ connected: boolean; has_cookie: boolean }>("get_namebase_status")
        .then((status) => {
          setNamebaseConnected(status.connected);
        })
        .catch(() => {});
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

  const isLocalhost =
    (form.hsd_wallet_api_url || "").includes("127.0.0.1") ||
    (form.hsd_wallet_api_url || "").includes("localhost");

  return (
    <div className="space-y-6 max-w-xl pb-16">
      <h2 className="text-xl font-bold">Settings</h2>

      <div className="bg-white rounded p-4 border border-gray-200 space-y-4">
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
      </div>

      <div className="bg-white rounded p-4 border border-gray-200 space-y-4">
        <h3 className="text-sm font-semibold text-gray-700">Security</h3>
        <div className="flex items-center justify-between">
          <div>
            <div className="text-sm font-medium">Write Mode</div>
            <div className="text-xs text-gray-500">
              Enable write actions (renewals, updates). Disabled by default.
            </div>
          </div>
          <Button
            variant={form.write_mode === "true" ? "danger" : "secondary"}
            size="sm"
            onClick={() => updateField("write_mode", form.write_mode === "true" ? "false" : "true")}
          >
            {form.write_mode === "true" ? "Enabled" : "Disabled"}
          </Button>
        </div>
        {form.write_mode === "true" && (
          <div className="bg-red-50 border border-red-200 rounded p-2 text-xs text-red-700">
            Write mode is enabled. Write actions will require confirmation dialogs. Use with caution on mainnet.
          </div>
        )}
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

      <div className="bg-white rounded p-4 border border-gray-200 space-y-2">
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

      <div className="bg-white rounded p-4 border border-gray-200 space-y-4">
        <h3 className="text-sm font-semibold text-gray-700">Namebase Connection</h3>
        <div className="text-xs text-gray-500">
          Connect to Namebase to import your TLDs. Paste your session cookie from browser DevTools.
        </div>
        <Input
          label="Session Cookie"
          type="password"
          value={namebaseCookie}
          onChange={(e) => setNamebaseCookie(e.target.value)}
          placeholder="Paste Namebase session cookie"
        />
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="primary"
            onClick={async () => {
              if (!namebaseCookie.trim()) return;
              setConnectingNamebase(true);
              try {
                await invoke("connect_namebase", { cookie: namebaseCookie });
                setNamebaseConnected(true);
                showToast("Connected to Namebase", "success");
              } catch (e) {
                showToast(mapError(e), "error");
              } finally {
                setConnectingNamebase(false);
              }
            }}
            disabled={!namebaseCookie.trim() || connectingNamebase}
          >
            {connectingNamebase ? "Connecting..." : "Connect"}
          </Button>
          {namebaseConnected && (
            <Button
              size="sm"
              variant="ghost"
              onClick={async () => {
                await invoke("disconnect_namebase");
                setNamebaseConnected(false);
                setNamebaseCookie("");
                showToast("Disconnected from Namebase", "success");
              }}
            >
              Disconnect
            </Button>
          )}
        </div>
        {namebaseConnected && (
          <div className="text-xs text-green-600">Connected to Namebase</div>
        )}
        <div className="text-xs text-gray-400">
          To get your cookie: Open Namebase in browser → F12 → Network tab → copy Cookie header from any request.
        </div>
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
