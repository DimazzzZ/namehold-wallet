import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Dialog } from "./ui/Dialog";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { Input } from "./ui/Input";
import { useSettingsStore } from "../stores/settings";
import { useWalletConnection } from "../queries/wallet";
import { useUiStore } from "../stores/ui";
import { mapError } from "../lib/errors";

interface WriteSetupWizardProps {
  open: boolean;
  onClose: () => void;
}

type StepKey = "explain" | "connect" | "test" | "enable";

const STEPS: { key: StepKey; label: string }[] = [
  { key: "explain", label: "Why" },
  { key: "connect", label: "Connect hsd" },
  { key: "test", label: "Test" },
  { key: "enable", label: "Enable" },
];

/**
 * Guided, progressive wizard that walks a read-only user through enabling
 * write mode by connecting a local hsd node. This replaces the expectation
 * that users discover node URLs and write_mode toggles by reading Settings.
 */
export function WriteSetupWizard({ open, onClose }: WriteSetupWizardProps) {
  const qc = useQueryClient();
  const settings = useSettingsStore((s) => s.settings);
  const saveAll = useSettingsStore((s) => s.saveAll);
  const showToast = useUiStore((s) => s.showToast);
  const { data: conn, isLoading: testing, refetch } = useWalletConnection();

  const [step, setStep] = useState<StepKey>("explain");
  const [walletApiUrl, setWalletApiUrl] = useState(
    settings?.hsd_wallet_api_url || "http://127.0.0.1:12039",
  );
  const [nodeApiUrl, setNodeApiUrl] = useState(
    settings?.hsd_node_api_url || "http://127.0.0.1:12037",
  );
  const [apiKey, setApiKey] = useState(settings?.hsd_api_key || "");
  const [enabling, setEnabling] = useState(false);

  const reset = () => {
    setStep("explain");
  };

  const handleClose = () => {
    reset();
    onClose();
  };

  const handleSaveConnection = async () => {
    try {
      await saveAll({
        hsd_wallet_api_url: walletApiUrl.trim(),
        hsd_node_api_url: nodeApiUrl.trim(),
        hsd_api_key: apiKey.trim(),
        connection_mode: "local_managed_hsd",
      });
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["read"] });
      setStep("test");
    } catch (e) {
      showToast(mapError(e), "error");
    }
  };

  const handleTest = async () => {
    await refetch();
  };

  const handleEnable = async () => {
    setEnabling(true);
    try {
      await saveAll({ write_mode: "true" });
      qc.invalidateQueries({ queryKey: ["wallet"] });
      qc.invalidateQueries({ queryKey: ["read"] });
      showToast("Write mode enabled", "success");
      handleClose();
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setEnabling(false);
    }
  };

  const currentIndex = STEPS.findIndex((s) => s.key === step);

  return (
    <Dialog open={open} onClose={handleClose} title="Enable write mode">
      <div className="space-y-4">
        {/* Step indicator */}
        <div className="flex items-center gap-2 text-xs">
          {STEPS.map((s, i) => (
            <div key={s.key} className="flex items-center gap-2">
              <span
                className={
                  i <= currentIndex
                    ? "font-medium text-blue-700"
                    : "text-gray-400"
                }
              >
                {i + 1}. {s.label}
              </span>
              {i < STEPS.length - 1 && <span className="text-gray-300">›</span>}
            </div>
          ))}
        </div>

        {step === "explain" && (
          <div className="space-y-3">
            <div className="bg-green-50 border border-green-200 rounded p-3 text-sm text-green-800">
              You are currently in safe <strong>read-only</strong> mode. You can
              view balances and names, but cannot send HNS or perform name
              operations.
            </div>
            <p className="text-sm text-gray-600">
              To send HNS, get a live receive address, and perform name
              operations (transfer, renew, finalize), you need to connect your
              own local <code>hsd</code> node with the wallet plugin enabled.
            </p>
            <ol className="text-sm text-gray-600 list-decimal list-inside space-y-1">
              <li>Install and start hsd locally</li>
              <li>Make sure the wallet plugin is enabled (no <code>--no-wallet</code>)</li>
              <li>Connect this app to your node</li>
              <li>Test the connection and enable write mode</li>
            </ol>
            <div className="flex justify-end">
              <Button variant="primary" onClick={() => setStep("connect")}>
                Get started
              </Button>
            </div>
          </div>
        )}

        {step === "connect" && (
          <div className="space-y-3">
            <p className="text-sm text-gray-600">
              Point the app at your running hsd node. Defaults match a standard
              local mainnet install.
            </p>
            <Input
              label="Wallet API URL"
              value={walletApiUrl}
              onChange={(e) => setWalletApiUrl(e.target.value)}
              placeholder="http://127.0.0.1:12039"
            />
            <Input
              label="Node API URL"
              value={nodeApiUrl}
              onChange={(e) => setNodeApiUrl(e.target.value)}
              placeholder="http://127.0.0.1:12037"
            />
            <Input
              label="API Key (optional)"
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder="Leave empty if not configured"
            />
            <div className="flex gap-2 justify-end">
              <Button variant="ghost" onClick={() => setStep("explain")}>
                Back
              </Button>
              <Button variant="primary" onClick={handleSaveConnection}>
                Save & continue
              </Button>
            </div>
          </div>
        )}

        {step === "test" && (
          <div className="space-y-3">
            <p className="text-sm text-gray-600">
              Test that the app can reach your hsd wallet.
            </p>
            <div className="bg-gray-50 rounded p-3 text-sm flex items-center justify-between">
              <span>Connection status</span>
              {testing ? (
                <Badge>Checking…</Badge>
              ) : conn?.connected ? (
                <Badge variant="success">Connected</Badge>
              ) : (
                <Badge variant="error">Not reachable</Badge>
              )}
            </div>
            {!conn?.connected && !testing && (
              <div className="bg-yellow-50 border border-yellow-200 rounded p-2 text-xs text-yellow-800">
                hsd wallet is not reachable. Verify hsd is running with the
                wallet plugin enabled, then test again.
              </div>
            )}
            <div className="flex gap-2 justify-end">
              <Button variant="ghost" onClick={() => setStep("connect")}>
                Back
              </Button>
              <Button variant="secondary" onClick={handleTest} disabled={testing}>
                {testing ? "Testing…" : "Test connection"}
              </Button>
              <Button
                variant="primary"
                onClick={() => setStep("enable")}
                disabled={!conn?.connected}
              >
                Continue
              </Button>
            </div>
          </div>
        )}

        {step === "enable" && (
          <div className="space-y-3">
            <div className="bg-red-50 border border-red-200 rounded p-3 text-sm text-red-700">
              Enabling write mode allows this app to spend HNS and modify names.
              Make sure you trust this machine and node.
            </div>
            <div className="flex gap-2 justify-end">
              <Button variant="ghost" onClick={() => setStep("test")}>
                Back
              </Button>
              <Button
                variant="danger"
                onClick={handleEnable}
                disabled={enabling}
              >
                {enabling ? "Enabling…" : "Enable write mode"}
              </Button>
            </div>
          </div>
        )}
      </div>
    </Dialog>
  );
}
