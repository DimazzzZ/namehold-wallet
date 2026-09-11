import type { NodeConnectionCheck } from "../../types";

interface Props {
  result: NodeConnectionCheck | null;
  error: string | null;
}

/** Inline outcome of a "Test connection" probe: a green summary or a red reason. */
export function ConnectionCheckStatus({ result, error }: Props) {
  if (result?.reachable) {
    return (
      <span className="text-xs text-green-600" data-testid="connection-success">
        ✓ Connected · height {result.height} · {result.synced ? "synced" : "syncing"}
        {result.network ? ` · ${result.network}` : ""}
        {result.networkMatches === false && (
          <span className="block text-amber-600" data-testid="connection-network-mismatch">
            Network mismatch — this wallet expects a different chain; reads and sends via this node
            will be refused.
          </span>
        )}
      </span>
    );
  }
  if (error) {
    return (
      <span className="text-xs text-red-600" data-testid="connection-error">
        {error}
      </span>
    );
  }
  return null;
}
