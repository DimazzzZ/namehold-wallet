import { useQueryClient } from "@tanstack/react-query";
import { useSettingsStore } from "../stores/settings";
import { AddWalletForm } from "./AddWalletForm";
import { useState } from "react";
import { Button } from "./ui/Button";
import { useNodeConnectionCheck } from "../hooks/useNodeConnectionCheck";
import { fromConnectionMode } from "../lib/connectionMode";
import { boolToSetting } from "../lib/settingsBool";
import { RemoteNodeFields } from "./ui/RemoteNodeFields";

/**
 * Wallet-first, non-custodial onboarding (first run, zero profiles).
 *
 * Secrets never touch React: creating or importing a wallet hands off to the
 * Rust-owned secure window. This screen is just the welcome chrome around the
 * shared `AddWalletForm`; on success it marks onboarding complete.
 */
export function Onboarding() {
  const qc = useQueryClient();
  const saveAll = useSettingsStore((s) => s.saveAll);
  const [step, setStep] = useState<"connection" | "wallet">("connection");

  const finish = async () => {
    await saveAll({ onboarding_complete: "true" });
    qc.invalidateQueries({ queryKey: ["wallet"] });
    qc.invalidateQueries({ queryKey: ["read"] });
  };

  if (step === "connection") {
    return <ConnectionChoice onNext={() => setStep("wallet")} />;
  }

  return (
    <div className="flex h-screen items-center justify-center bg-gray-100 p-6">
      <div className="bg-white rounded-lg shadow-lg max-w-lg w-full p-8">
        <h1 className="text-2xl font-bold text-gray-900 mb-2">Welcome to Namehold</h1>
        <p className="text-gray-500 mb-6">
          A non-custodial wallet for moving and managing Handshake names. Your keys never leave this
          device, and your recovery phrase is only ever shown in a secure window.
        </p>
        <AddWalletForm defaultLabel="Primary" onDone={finish} />
      </div>
    </div>
  );
}

/**
 * First-run connection choice: "How do you want to connect?"
 * Three options: Local full node (default), Remote node, or SPV.
 * Persists chain_source, node_mode, and node_rpc_url, then advances to wallet creation.
 */
function ConnectionChoice({ onNext }: { onNext: () => void }) {
  const saveAll = useSettingsStore((s) => s.saveAll);
  const [remoteUrl, setRemoteUrl] = useState("");
  const [remoteApiKey, setRemoteApiKey] = useState("");
  const [allowRemoteBroadcast, setAllowRemoteBroadcast] = useState(false);
  const probe = useNodeConnectionCheck();

  const selectLocal = async () => {
    await saveAll(fromConnectionMode("local_full"));
    onNext();
  };

  const selectSpv = async () => {
    await saveAll(fromConnectionMode("local_spv"));
    onNext();
  };

  const selectRemote = async () => {
    // The Continue button is disabled until a probe succeeds; this guard only
    // covers a programmatic click.
    if (!probe.ok) return;
    await saveAll({
      ...fromConnectionMode("remote_node"),
      node_rpc_url: remoteUrl.trim(),
      node_rpc_api_key: remoteApiKey,
      allow_remote_broadcast: boolToSetting(allowRemoteBroadcast),
    });
    onNext();
  };

  return (
    <div className="flex h-screen items-center justify-center bg-gray-100 p-6">
      <div className="bg-white rounded-lg shadow-lg max-w-2xl w-full p-8">
        <h1 className="text-2xl font-bold text-gray-900 mb-2">How do you want to connect?</h1>
        <p className="text-gray-500 mb-6">
          Your keys stay on this device. Choose how the wallet reads and sends transactions. Remote
          and SPV are a privacy/trust tradeoff, not custody — your recovery phrase never leaves this
          device.
        </p>

        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          {/* Local full node */}
          <button
            onClick={selectLocal}
            className="border-2 border-gray-300 rounded-lg p-4 text-left hover:border-blue-500 hover:bg-blue-50 transition"
            data-testid="select-local-button"
          >
            <h3 className="font-bold text-gray-900">Local Full Node</h3>
            <p className="text-xs text-gray-600 mt-2">
              Start hsd on this device. ~15GB chain, full indexes. Best privacy.
            </p>
          </button>

          {/* Remote node */}
          <div className="border-2 border-gray-300 rounded-lg p-4 hover:border-blue-500 transition">
            <h3 className="font-bold text-gray-900">Remote Node</h3>
            <p className="text-xs text-gray-600 mt-2">
              Point to a remote hsd RPC. Fast setup, no local chain. Keys stay local.
            </p>
            <div className="mt-3 space-y-2">
              <RemoteNodeFields
                url={remoteUrl}
                apiKey={remoteApiKey}
                onUrlChange={setRemoteUrl}
                onApiKeyChange={setRemoteApiKey}
                probe={probe}
                urlPlaceholder="https://node.example.com:12037"
                apiKeyPlaceholder="API key (optional)"
                urlTestId="remote-url-input"
                apiKeyTestId="remote-api-key-input"
                actionsLayout="stack"
              />
              <label className="flex items-center gap-2 text-xs">
                <input
                  type="checkbox"
                  checked={allowRemoteBroadcast}
                  onChange={(e) => setAllowRemoteBroadcast(e.target.checked)}
                  data-testid="allow-remote-broadcast-checkbox"
                />
                Allow sending via remote node
              </label>
              <Button
                size="sm"
                onClick={selectRemote}
                disabled={!probe.ok}
                data-testid="select-remote-button"
              >
                Continue
              </Button>
            </div>
          </div>

          {/* SPV */}
          <button
            onClick={selectSpv}
            className="border-2 border-gray-300 rounded-lg p-4 text-left hover:border-blue-500 hover:bg-blue-50 transition"
            data-testid="select-spv-button"
          >
            <h3 className="font-bold text-gray-900">SPV (Lightweight)</h3>
            <p className="text-xs text-gray-600 mt-2">
              Headers only, ~1MB. Fast sync. Uses explorer for data. Read-only.
            </p>
          </button>
        </div>
      </div>
    </div>
  );
}
