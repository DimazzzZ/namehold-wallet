import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { useWalletAddress } from "../queries/wallet";
import { useReadContext } from "../queries/read";
import { canWrite } from "../lib/providerMode";
import { cn } from "../lib/utils";

interface NamebaseStatus {
  connected: boolean;
  has_cookie: boolean;
  error?: string;
}

type StepState = "done" | "active" | "todo";

interface AssistantStep {
  key: string;
  title: string;
  description: string;
  state: StepState;
  hint?: string;
}

/**
 * A guided, linear overview of the Namebase → self-custody migration.
 *
 * This sits above the detailed Namebase dashboard and gives users a clear
 * sense of "where am I and what's next" without hiding the underlying tools.
 * Step state is derived from real data (Namebase connection, wallet receive
 * address availability, on-chain write capability) rather than local flags so
 * the checklist always reflects reality.
 */
export function MigrationAssistant() {
  const { data: nbStatus } = useQuery({
    queryKey: ["namebase", "status"],
    queryFn: () =>
      invoke<NamebaseStatus>("get_namebase_status"),
    retry: false,
  });
  const { data: walletAddress } = useWalletAddress();
  const { data: readContext } = useReadContext();

  const connected = nbStatus?.connected ?? false;
  const hasReceiveAddress = Boolean(walletAddress);
  const writeReady = readContext ? canWrite(readContext) : false;

  const steps: AssistantStep[] = useMemo(() => {
    // Step 1 — connect Namebase (read your custodial holdings).
    const connectState: StepState = connected ? "done" : "active";

    // Step 2 — have a destination address to receive names.
    const receiveState: StepState = !connected
      ? "todo"
      : hasReceiveAddress
        ? "done"
        : "active";

    // Step 3 — transfer names out of Namebase to your address.
    const transferState: StepState =
      connected && hasReceiveAddress ? "active" : "todo";

    // Step 4 — verify ownership on-chain (and finalize when writable).
    const verifyState: StepState = "todo";

    return [
      {
        key: "connect",
        title: "Connect Namebase",
        description:
          "Paste your Namebase session cookie below to read your custodial domains.",
        state: connectState,
      },
      {
        key: "receive",
        title: "Prepare a receive address",
        description: hasReceiveAddress
          ? "Your wallet address is ready to receive transferred names."
          : "Import or select a wallet so we have an address to transfer names to.",
        state: receiveState,
        hint: !hasReceiveAddress
          ? "No wallet address yet — set one up from the Wallet screen."
          : undefined,
      },
      {
        key: "transfer",
        title: "Transfer names to your wallet",
        description:
          "Select the names you control and initiate transfers to your address.",
        state: transferState,
        hint: "Staked names cannot be transferred until unstaked.",
      },
      {
        key: "verify",
        title: "Verify on-chain ownership",
        description: writeReady
          ? "Once transfers confirm, reconcile and finalize ownership in Sync & Verify."
          : "Once transfers confirm, reconcile against your inventory in Sync & Verify.",
        state: verifyState,
        hint: !writeReady
          ? "Finalizing on-chain requires write mode (a local hsd node)."
          : undefined,
      },
    ];
  }, [connected, hasReceiveAddress, writeReady]);

  const doneCount = steps.filter((s) => s.state === "done").length;

  return (
    <div className="bg-white rounded-lg border border-gray-200 p-5 mb-6">
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-sm font-semibold text-gray-900">Migration assistant</h3>
          <p className="text-xs text-gray-500">
            Move your names from Namebase custody into your own wallet, step by step.
          </p>
        </div>
        <span className="text-xs text-gray-500">
          {doneCount}/{steps.length} complete
        </span>
      </div>

      <ol className="space-y-3">
        {steps.map((step, idx) => (
          <li key={step.key} className="flex gap-3">
            <div
              className={cn(
                "flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-xs font-medium",
                step.state === "done" && "bg-green-100 text-green-700",
                step.state === "active" && "bg-blue-600 text-white",
                step.state === "todo" && "bg-gray-100 text-gray-400",
              )}
            >
              {step.state === "done" ? "✓" : idx + 1}
            </div>
            <div className="min-w-0">
              <div
                className={cn(
                  "text-sm font-medium",
                  step.state === "todo" ? "text-gray-400" : "text-gray-900",
                )}
              >
                {step.title}
              </div>
              <div
                className={cn(
                  "text-xs",
                  step.state === "todo" ? "text-gray-400" : "text-gray-500",
                )}
              >
                {step.description}
              </div>
              {step.hint && (
                <div className="mt-0.5 text-[11px] text-amber-600">{step.hint}</div>
              )}
            </div>
          </li>
        ))}
      </ol>
    </div>
  );
}
