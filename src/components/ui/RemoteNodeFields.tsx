import { Input } from "./Input";
import { Button } from "./Button";
import { ConnectionCheckStatus } from "./ConnectionCheckStatus";
import type { NodeConnectionCheckState } from "../../hooks/useNodeConnectionCheck";

export interface RemoteNodeFieldsProps {
  /** RPC URL value. */
  url: string;
  /** RPC API key value. */
  apiKey: string;
  /** Called when the URL changes. The probe is reset for you. */
  onUrlChange: (url: string) => void;
  /** Called when the API key changes. The probe is reset for you. */
  onApiKeyChange: (key: string) => void;
  /** The probe state (testing, result, error, ok). */
  probe: NodeConnectionCheckState;
  /** Label above the URL input. Omit for a placeholder-only field. */
  urlLabel?: string;
  /** Label above the API key input. Omit for a placeholder-only field. */
  apiKeyLabel?: string;
  /** Placeholder for the URL input. */
  urlPlaceholder?: string;
  /** Placeholder for the API key input. */
  apiKeyPlaceholder?: string;
  /** testid for the URL input. */
  urlTestId?: string;
  /** testid for the API key input. */
  apiKeyTestId?: string;
  /**
   * How the Test button and its status line sit together: side by side
   * ("row", Settings) or stacked in the parent's own vertical rhythm
   * ("stack", onboarding).
   */
  actionsLayout?: "row" | "stack";
}

/**
 * Shared remote-node connection fields: RPC URL + API key + Test button.
 * Used by both Settings and onboarding to avoid duplicating the input cluster.
 * Labels, placeholders, testids, and the Test-button layout are the parent's
 * to choose; everything else is identical between the two screens.
 *
 * Editing either field resets the probe here, so a stale "connected" result
 * can never outlive the URL or key it was probed with. Leaving that to the
 * caller made it a footgun: two call sites had to remember it, and forgetting
 * it in a third would silently show a green check for an unprobed node.
 */
export function RemoteNodeFields({
  url,
  apiKey,
  onUrlChange,
  onApiKeyChange,
  probe,
  urlLabel,
  apiKeyLabel,
  urlPlaceholder,
  apiKeyPlaceholder,
  urlTestId = "node-rpc-url-input",
  apiKeyTestId = "node-rpc-api-key-input",
  actionsLayout = "row",
}: RemoteNodeFieldsProps) {
  return (
    <>
      <Input
        label={urlLabel}
        value={url}
        onChange={(e) => {
          probe.reset();
          onUrlChange(e.target.value);
        }}
        placeholder={urlPlaceholder}
        data-testid={urlTestId}
      />
      <Input
        label={apiKeyLabel}
        type="password"
        value={apiKey}
        onChange={(e) => {
          probe.reset();
          onApiKeyChange(e.target.value);
        }}
        placeholder={apiKeyPlaceholder}
        data-testid={apiKeyTestId}
      />
      <div className={actionsLayout === "row" ? "flex items-center gap-2" : "space-y-2"}>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => probe.run(url, apiKey)}
          disabled={probe.testing}
          data-testid="test-connection-button"
        >
          {probe.testing ? "Testing…" : "Test connection"}
        </Button>
        <ConnectionCheckStatus result={probe.result} error={probe.error} />
      </div>
    </>
  );
}
