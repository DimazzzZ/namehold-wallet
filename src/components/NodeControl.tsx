import { useState, useEffect, useRef } from "react";
import { useNodeStatus, useStopHsd, useStartHsd, useHsdLog } from "../queries/node";
import { useReadContext } from "../queries/read";
import { canManageNode } from "../lib/providerMode";
import { useSettingsStore } from "../stores/settings";
import { useUiStore } from "../stores/ui";
import { Button } from "./ui/Button";
import { Input } from "./ui/Input";
import { StickyFooter } from "./ui/StickyFooter";
import { mapError } from "../lib/errors";
import { open } from "@tauri-apps/plugin-dialog";

export function NodeControl() {
  const { data: status, isLoading, refetch } = useNodeStatus();
  const { data: readContext } = useReadContext();
  const stopHsd = useStopHsd();
  const startHsd = useStartHsd();
  const hsdLog = useHsdLog(200);
  const [showLog, setShowLog] = useState(false);
  const settings = useSettingsStore((s) => s.settings);
  const saveAll = useSettingsStore((s) => s.saveAll);
  const showToast = useUiStore((s) => s.showToast);

  const [prefix, setPrefix] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [network, setNetwork] = useState("");
  const [showApiKey, setShowApiKey] = useState(false);
  const [saving, setSaving] = useState(false);
  const [starting, setStarting] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  const [startWarning, setStartWarning] = useState<string | null>(null);
  const [configDirty, setConfigDirty] = useState(false);

  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Track how long we've been starting and whether the hsd log keeps growing.
  const startElapsedRef = useRef(0);
  const lastLogCountRef = useRef(0);
  const stalledTicksRef = useRef(0);

  // Absolute safety cap (5 minutes) and how long with no log progress before
  // we treat the start as failed.
  const HARD_TIMEOUT_MS = 300_000;
  const POLL_INTERVAL_MS = 2000;
  // Number of consecutive polls (~no log growth) before declaring failure.
  const MAX_STALLED_TICKS = 30; // ~60s of no progress

  const defaultPrefix = settings?.hsd_prefix || "";
  const defaultApiKey = settings?.hsd_api_key || "";
  const defaultNetwork = settings?.hsd_network || "mainnet";

  // Provider-aware node management: only the local managed hsd provider can be
  // started/stopped from here. Remote / external providers are not manageable.
  const nodeManageable = canManageNode(readContext);
  const activeProvider = readContext?.activeReadProvider;

  const isRunning = status?.running ?? false;
  const chain = status?.chain;
  const hsdVersion = status?.hsd_version;
  const hsdBinary = status?.hsd_binary;
  const hsdBinaryFound = status?.hsd_binary_found;

  // hsd getblockchaininfo returns 'blocks', 'headers', 'verificationprogress'
  const chainHeight = (chain?.blocks as number) || 0;
  // Use hsd's verificationprogress if available (0.0 to 1.0), otherwise estimate
  const verificationProgress = chain?.verificationprogress as number | undefined;
  const isSyncing = verificationProgress != null ? verificationProgress < 0.999 : chainHeight < 340_000;
  const progressPct = verificationProgress != null
    ? Math.min(verificationProgress * 100, 100)
    : Math.min((chainHeight / 340_000) * 100, 99.9);

  // Poll for status when starting. Instead of a fixed 30s timeout, we keep
  // waiting as long as the captured hsd log is still growing (hsd is
  // initializing - this matters for large chains or external/slow drives).
  useEffect(() => {
    if (starting && !isRunning) {
      startElapsedRef.current = 0;
      lastLogCountRef.current = 0;
      stalledTicksRef.current = 0;
      setStartWarning(null);

      const fail = (msg: string) => {
        setStarting(false);
        setStartError(msg);
        if (pollRef.current) clearInterval(pollRef.current);
        if (timeoutRef.current) clearTimeout(timeoutRef.current);
        // Auto-fetch the captured hsd log so the user can see the real error.
        setShowLog(true);
        hsdLog.refetch();
      };

      pollRef.current = setInterval(async () => {
        startElapsedRef.current += POLL_INTERVAL_MS;

        const result = await refetch();
        if (result.data?.running) {
          setStarting(false);
          setStartError(null);
          setStartWarning(null);
          if (pollRef.current) clearInterval(pollRef.current);
          if (timeoutRef.current) clearTimeout(timeoutRef.current);
          return;
        }

        // hsd not reporting healthy yet - check whether it's still making
        // progress by watching the captured log file grow.
        const logResult = await hsdLog.refetch();
        const lineCount = logResult.data?.exists ? logResult.data.lines.length : 0;

        if (lineCount > lastLogCountRef.current) {
          // Log is growing => hsd is still initializing. Keep waiting.
          lastLogCountRef.current = lineCount;
          stalledTicksRef.current = 0;
          if (startElapsedRef.current >= 30_000) {
            setStartWarning(
              "hsd is taking longer than usual to become ready (still initializing). This can happen on large chains or external/slow drives.",
            );
          }
        } else {
          // No new log output since last poll.
          stalledTicksRef.current += 1;
          if (stalledTicksRef.current >= MAX_STALLED_TICKS) {
            fail(
              "hsd stopped making progress and is not responding. Check the log below for errors.",
            );
          }
        }
      }, POLL_INTERVAL_MS);

      // Absolute safety cap so we never poll forever.
      timeoutRef.current = setTimeout(() => {
        fail(
          "hsd did not become ready within 5 minutes. Check the log below, or leave hsd running and click Refresh.",
        );
      }, HARD_TIMEOUT_MS);
    }

    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, [starting, isRunning, refetch]);

  // If hsd becomes running while we're starting, clear starting state
  useEffect(() => {
    if (isRunning && starting) {
      setStarting(false);
      setStartError(null);
      setStartWarning(null);
    }
  }, [isRunning, starting]);

  const handleSaveConfig = async () => {
    setSaving(true);
    try {
      const toSave: Record<string, string> = {};
      if (prefix) toSave.hsd_prefix = prefix;
      if (apiKey) toSave.hsd_api_key = apiKey;
      if (network) toSave.hsd_network = network;
      if (Object.keys(toSave).length > 0) {
        await saveAll(toSave);
        showToast("Configuration saved", "success");
      }
    } catch (e) {
      showToast(mapError(e), "error");
    } finally {
      setSaving(false);
    }
  };

  const handleStop = async () => {
    setStopping(true);
    try {
      const result = await stopHsd.mutateAsync();
      showToast(result, "success");
      setStartError(null);
      // Poll until stopped
      const checkInterval = setInterval(async () => {
        const r = await refetch();
        if (!r.data?.running) {
          setStopping(false);
          clearInterval(checkInterval);
        }
      }, 2000);
      setTimeout(() => {
        setStopping(false);
        clearInterval(checkInterval);
      }, 15000);
    } catch (e) {
      setStopping(false);
      showToast(mapError(e), "error");
    }
  };

  const handleStart = async () => {
    setStarting(true);
    setStartError(null);
    setStartWarning(null);
    try {
      const result = await startHsd.mutateAsync({
        prefix: prefix || defaultPrefix || undefined,
        api_key: apiKey || defaultApiKey || undefined,
        network: network || defaultNetwork || undefined,
      });
      showToast(result, "success");
      // Polling will handle the rest via useEffect
    } catch (e) {
      setStarting(false);
      setStartError(mapError(e));
    }
  };

  const statusColor = starting
    ? "bg-yellow-500"
    : stopping
      ? "bg-orange-500"
      : isRunning
        ? "bg-green-500"
        : "bg-red-500";

  const statusText = starting
    ? "Starting..."
    : stopping
      ? "Stopping..."
      : isRunning
        ? "Running"
        : "Stopped";

  return (
    <div className="space-y-6">
      <h2 className="text-xl font-bold">Node Control</h2>

      {/* Provider context: node controls only apply to a local managed hsd. */}
      {readContext && !nodeManageable && (
        <div className="bg-blue-50 border border-blue-200 rounded p-3 text-sm text-blue-800">
          <div>
            Active data source: <strong>{activeProvider?.label ?? "—"}</strong>
          </div>
          <div className="mt-1 text-blue-700">
            This provider is not managed by Namehold, so the node cannot be
            started or stopped here
            {activeProvider?.reason ? `: ${activeProvider.reason}` : "."}
          </div>
        </div>
      )}

      {/* Status */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <h3 className="text-sm font-semibold mb-3">Status</h3>
        {isLoading ? (
          <div className="text-gray-500">Checking...</div>
        ) : (
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <div className={`w-3 h-3 rounded-full ${statusColor} ${starting || stopping ? "animate-pulse" : ""}`} />
              <span className="font-medium">{statusText}</span>
              {starting && (
                <svg className="animate-spin h-4 w-4 text-yellow-500" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
              )}
            </div>

            {starting && (
              <div className="text-sm text-yellow-700 bg-yellow-50 rounded p-2 space-y-2">
                <div>
                  {startWarning ??
                    "hsd is starting up. This may take a few seconds while the node initializes..."}
                </div>
                <button
                  className="text-xs text-blue-600 hover:underline"
                  onClick={() => {
                    setShowLog((v) => !v);
                    if (!showLog) hsdLog.refetch();
                  }}
                >
                  {showLog ? "Hide hsd log" : "Show hsd log"}
                </button>
              </div>
            )}

            {startError && !starting && (
              <div className="text-sm text-red-600 bg-red-50 rounded p-2 space-y-2">
                <div>{startError}</div>
                <button
                  className="text-xs text-blue-600 hover:underline"
                  onClick={() => {
                    setShowLog((v) => !v);
                    if (!showLog) hsdLog.refetch();
                  }}
                >
                  {showLog ? "Hide hsd log" : "Show hsd log"}
                </button>
              </div>
            )}

            {showLog && (
              <div className="bg-gray-900 text-gray-100 rounded p-3 text-xs font-mono overflow-auto max-h-64">
                <div className="flex justify-between items-center mb-2 text-gray-400">
                  <span className="truncate">{hsdLog.data?.path || "hsd log"}</span>
                  <button
                    className="text-blue-300 hover:underline ml-2 shrink-0"
                    onClick={() => hsdLog.refetch()}
                  >
                    Refresh
                  </button>
                </div>
                {hsdLog.isFetching ? (
                  <div className="text-gray-400">Loading log...</div>
                ) : hsdLog.data && !hsdLog.data.exists ? (
                  <div className="text-gray-400">
                    No log file found. It is created the next time you start hsd from here.
                  </div>
                ) : hsdLog.data && hsdLog.data.lines.length === 0 ? (
                  <div className="text-gray-400">Log file is empty.</div>
                ) : (
                  <pre className="whitespace-pre-wrap break-all">
                    {hsdLog.data?.lines.join("\n")}
                  </pre>
                )}
              </div>
            )}

            {hsdVersion && (
              <div className="text-sm text-gray-600">
                <span className="text-gray-500">Version:</span> {hsdVersion}
              </div>
            )}

            {hsdBinary && (
              <div className="text-sm text-gray-600">
                <span className="text-gray-500">Binary:</span>{" "}
                <span className="font-mono text-xs">{hsdBinary}</span>
                {!hsdBinaryFound && (
                  <span className="text-red-500 text-xs ml-2">Not found</span>
                )}
              </div>
            )}

            {isRunning && chain && (
              <div className="space-y-2">
                <div className="text-sm text-gray-600">
                  <span className="text-gray-500">Block Height:</span>{" "}
                  {chainHeight.toLocaleString()}
                  {isSyncing && (
                    <span className="text-yellow-600 ml-2 text-xs">(syncing...)</span>
                  )}
                </div>
                <div>
                  <div className="flex justify-between text-sm mb-1">
                    <span className="text-gray-500">{isSyncing ? "Sync Progress" : "Synced"}</span>
                    <span className="font-mono">{progressPct.toFixed(1)}%</span>
                  </div>
                  <div className="w-full bg-gray-200 rounded-full h-2">
                    <div
                      className={`h-2 rounded-full transition-all ${isSyncing ? "bg-yellow-500" : "bg-green-500"}`}
                      style={{ width: `${Math.min(progressPct, 100)}%` }}
                    />
                  </div>
                </div>
              </div>
            )}

            {!isRunning && !starting && (
              <div className="text-sm text-gray-500">
                hsd is not running. Start it from the controls below.
              </div>
            )}
          </div>
        )}
      </div>

      {/* Controls */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <h3 className="text-sm font-semibold mb-3">Controls</h3>
        <div className="flex gap-3">
          <Button
            variant="primary"
            onClick={handleStart}
            disabled={!nodeManageable || isRunning || starting || startHsd.isPending}
          >
            {starting ? (
              <span className="flex items-center gap-2">
                <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
                Starting...
              </span>
            ) : startHsd.isPending ? (
              "Sending..."
            ) : (
              "Start hsd"
            )}
          </Button>
          <Button
            variant="danger"
            onClick={handleStop}
            disabled={!nodeManageable || !isRunning || stopping || stopHsd.isPending}
          >
            {stopping ? (
              <span className="flex items-center gap-2">
                <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
                Stopping...
              </span>
            ) : "Stop hsd"}
          </Button>
        </div>
      </div>

      {/* Current Configuration */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <h3 className="text-sm font-semibold mb-3">Current Configuration</h3>
        <div className="space-y-2 text-sm">
          <div className="flex justify-between">
            <span className="text-gray-500">Data Directory:</span>
            <span className="font-mono">{defaultPrefix || "~/.hsd (default)"}</span>
          </div>
          <div className="flex justify-between items-center">
            <span className="text-gray-500">API Key:</span>
            <div className="flex items-center gap-2">
              <span className="font-mono">
                {defaultApiKey ? (showApiKey ? defaultApiKey : "••••••••") : "Not set"}
              </span>
              {defaultApiKey && (
                <button
                  className="text-xs text-blue-600 hover:underline"
                  onClick={() => setShowApiKey(!showApiKey)}
                >
                  {showApiKey ? "Hide" : "Show"}
                </button>
              )}
            </div>
          </div>
          <div className="flex justify-between">
            <span className="text-gray-500">Network:</span>
            <span>{defaultNetwork}</span>
          </div>
        </div>
      </div>

      {/* Configuration Override */}
      <div className="bg-white rounded p-4 border border-gray-200">
        <h3 className="text-sm font-semibold mb-3">Update Configuration</h3>
        <p className="text-xs text-gray-500 mb-3">
          Change settings and click Save. Leave empty to keep current values.
        </p>
        <div className="space-y-3">
          <div className="flex gap-2 items-end">
            <div className="flex-1">
          <Input
            label="Data Directory (prefix)"
            value={prefix}
            onChange={(e) => { setPrefix(e.target.value); setConfigDirty(true); }}
            placeholder={defaultPrefix || "~/.hsd"}
          />
            </div>
            <Button
              size="sm"
              onClick={async () => {
                const selected = await open({ directory: true, multiple: false });
                if (selected) setPrefix(selected as string);
              }}
            >
              Browse
            </Button>
          </div>
          <Input
            label="API Key"
            type="password"
            value={apiKey}
            onChange={(e) => { setApiKey(e.target.value); setConfigDirty(true); }}
            placeholder={defaultApiKey ? "Using saved key" : "Enter API key"}
          />
          <Select
            label="Network"
            options={[
              { value: "", label: `Keep current (${defaultNetwork})` },
              { value: "mainnet", label: "Mainnet" },
              { value: "testnet", label: "Testnet" },
              { value: "regtest", label: "Regtest" },
            ]}
            value={network}
            onChange={(e) => { setNetwork(e.target.value); setConfigDirty(true); }}
          />
        </div>
      </div>

      {/* Info */}
      <div className="bg-white rounded p-4 border border-gray-200 text-sm text-gray-600 pb-16">
        <h3 className="font-semibold mb-2">About hsd</h3>
        <p>
          hsd is the Handshake full node software. It stores the blockchain and manages your wallet.
          The node must be running for Namehold to connect to your wallet.
        </p>
        <p className="mt-2">
          <strong>Data directory:</strong> Where hsd stores blockchain and wallet data.
          Use an external drive for large blockchain data.
        </p>
        <p className="mt-2">
          <strong>API Key:</strong> Required for authentication. Must match the key used when starting hsd.
        </p>
      </div>

      <StickyFooter>
        <span className="text-sm text-gray-500">
          {configDirty ? "You have unsaved configuration changes" : "Configuration up to date"}
        </span>
        <Button variant="primary" onClick={handleSaveConfig} disabled={saving || !configDirty}>
          {saving ? "Saving..." : "Save Configuration"}
        </Button>
      </StickyFooter>
    </div>
  );
}

function Select({ label, options, value, onChange }: {
  label: string;
  options: { value: string; label: string }[];
  value: string;
  onChange: (e: React.ChangeEvent<HTMLSelectElement>) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <label className="text-sm font-medium text-gray-700">{label}</label>
      <select
        className="border border-gray-300 rounded px-3 py-1.5 text-sm bg-white focus:outline-none focus:ring-2 focus:ring-blue-500"
        value={value}
        onChange={onChange}
      >
        {options.map((opt) => (
          <option key={opt.value} value={opt.value}>{opt.label}</option>
        ))}
      </select>
    </div>
  );
}
