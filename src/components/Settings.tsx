import { useSettingsStore } from "../stores/settings";
import { Input } from "./ui/Input";
import { Select } from "./ui/Select";
import { Button } from "./ui/Button";
import { useUiStore } from "../stores/ui";

export function Settings() {
  const { settings, update, loaded, passphrase, setPassphrase, clearPassphrase } = useSettingsStore();
  const showToast = useUiStore((s) => s.showToast);

  if (!loaded || !settings) {
    return <div className="text-gray-500">Loading settings...</div>;
  }

  const isLocalhost =
    settings.hsd_wallet_api_url.includes("127.0.0.1") ||
    settings.hsd_wallet_api_url.includes("localhost");

  const handleSave = async (key: string, value: string) => {
    try {
      await update(key, value);
      showToast("Setting saved", "success");
    } catch (e) {
      showToast(`Failed to save: ${e}`, "error");
    }
  };

  return (
    <div className="space-y-6 max-w-xl">
      <h2 className="text-xl font-bold">Settings</h2>

      <div className="bg-white rounded p-4 border border-gray-200 space-y-4">
        <h3 className="text-sm font-semibold text-gray-700">Handshake Node</h3>
        <Input
          label="Wallet API URL"
          value={settings.hsd_wallet_api_url}
          onChange={(e) => update("hsd_wallet_api_url", e.target.value)}
          onBlur={(e) => handleSave("hsd_wallet_api_url", e.target.value)}
        />
        {!isLocalhost && (
          <div className="bg-red-50 border border-red-200 rounded p-2 text-xs text-red-700">
            Warning: Non-localhost URL. Only use local connections for security.
          </div>
        )}
        <Input
          label="Node API URL"
          value={settings.hsd_node_api_url}
          onChange={(e) => update("hsd_node_api_url", e.target.value)}
          onBlur={(e) => handleSave("hsd_node_api_url", e.target.value)}
        />
        <Input
          label="API Key"
          type="password"
          value={settings.hsd_api_key}
          onChange={(e) => update("hsd_api_key", e.target.value)}
          onBlur={(e) => handleSave("hsd_api_key", e.target.value)}
          placeholder="Leave empty if no auth"
        />
        <Input
          label="Wallet ID"
          value={settings.hsd_wallet_id}
          onChange={(e) => update("hsd_wallet_id", e.target.value)}
          onBlur={(e) => handleSave("hsd_wallet_id", e.target.value)}
        />
        <Select
          label="Network"
          options={[
            { value: "mainnet", label: "Mainnet" },
            { value: "testnet", label: "Testnet" },
            { value: "regtest", label: "Regtest" },
          ]}
          value={settings.hsd_network}
          onChange={(e) => handleSave("hsd_network", e.target.value)}
        />
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
            variant={settings.write_mode === "true" ? "danger" : "secondary"}
            size="sm"
            onClick={() =>
              handleSave(
                "write_mode",
                settings.write_mode === "true" ? "false" : "true",
              )
            }
          >
            {settings.write_mode === "true" ? "Enabled" : "Disabled"}
          </Button>
        </div>
        {settings.write_mode === "true" && (
          <div className="bg-red-50 border border-red-200 rounded p-2 text-xs text-red-700">
            Write mode is enabled. Write actions will require confirmation dialogs. Use with caution
            on mainnet.
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
    </div>
  );
}
