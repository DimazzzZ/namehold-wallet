import { Input } from "./Input";
import { Button } from "./Button";
import { ConnectionCheckStatus } from "./ConnectionCheckStatus";
import type { NodeConnectionCheckState } from "../../hooks/useNodeConnectionCheck";

export interface RemoteNodeFieldsProps {
  /** RPC URL value */
  url: string;
  /** RPC API key value */
  apiKey: string;
  /** Called when URL changes; caller should reset probe */
  onUrlChange: (url: string) => void;
  /** Called when API key changes; caller should reset probe */
  onApiKeyChange: (key: string) => void;
  /** The probe state (testing, result, error, ok) */
  probe: NodeConnectionCheckState;
  /** Label for the URL input; if omitted, uses placeholder-only style */
  urlLabel?: string;
  /** Placeholder for URL input */
  urlPlaceholder?: string;
  /** Placeholder for API key input; if omitted, defaults to "(optional)" */
  apiKeyPlaceholder?: string;
  /** testid for the URL input */
  urlTestId?: string;
  /** testid for the API key input */
  apiKeyTestId?: string;
  /** testid for the Test button */
  buttonTestId?: string;
  /** Idle label for the Test button (default "Test Connection") */
  buttonLabel?: string;
  /** Busy label for the Test button (default "Testing…") */
  buttonBusyLabel?: string;
}

/**
 * Shared remote-node connection fields: RPC URL + API key + Test button.
 * Used by both Settings and Onboarding to avoid duplicating the input cluster.
 * The caller is responsible for:
 * - Calling `probe.reset()` when the URL or key changes (via onUrlChange/onApiKeyChange)
 * - Wrapping this in a form or container with appropriate labels/help text
 */
export function RemoteNodeFields({
  url,
  apiKey,
  onUrlChange,
  onApiKeyChange,
  probe,
  urlLabel,
  urlPlaceholder = "https://node.example.com:12037",
  apiKeyPlaceholder = "(optional)",
  urlTestId = "node-rpc-url-input",
  apiKeyTestId = "node-rpc-api-key-input",
  buttonTestId = "test-connection-button",
  buttonLabel = "Test Connection",
  buttonBusyLabel = "Testing…",
}: RemoteNodeFieldsProps) {
  return (
    <>
      <Input
        label={urlLabel}
        value={url}
        onChange={(e) => onUrlChange(e.target.value)}
        placeholder={urlPlaceholder}
        data-testid={urlTestId}
      />
      <Input
        label={urlLabel ? "Node RPC API key" : undefined}
        type="password"
        value={apiKey}
        onChange={(e) => onApiKeyChange(e.target.value)}
        placeholder={apiKeyPlaceholder}
        data-testid={apiKeyTestId}
      />
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          variant="secondary"
          onClick={() => probe.run(url, apiKey)}
          disabled={probe.testing}
          data-testid={buttonTestId}
        >
          {probe.testing ? buttonBusyLabel : buttonLabel}
        </Button>
        <ConnectionCheckStatus result={probe.result} error={probe.error} />
      </div>
    </>
  );
}
