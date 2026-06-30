import { Badge } from "./Badge";
import { providerLabel, providerTone } from "../../lib/providerMode";
import type { ReadContext, ReadProviderKind, StatusTone } from "../../types";

/**
 * Small shared badge/pill describing the active read source — `Local hsd`,
 * `Remote hsd`, or `External read-only` (HNSFans). Can be driven either by a
 * full `ReadContext` (preferred — colors reflect health/fallback) or by an
 * explicit provider `kind` for simpler call sites.
 */

const TONE_TO_VARIANT: Record<
  StatusTone,
  "default" | "success" | "warning" | "error" | "info"
> = {
  default: "default",
  info: "info",
  success: "success",
  warning: "warning",
  error: "error",
};

interface SourceBadgeProps {
  /** Resolved read context (preferred). Drives label, tone, and read-only hint. */
  context?: ReadContext | null;
  /** Explicit provider kind, used when no context is available. */
  kind?: ReadProviderKind;
  /** Optional remote label override (e.g. "Home server"). */
  remoteLabel?: string | null;
  className?: string;
}

export function SourceBadge({
  context,
  kind,
  remoteLabel,
  className,
}: SourceBadgeProps) {
  const resolvedKind: ReadProviderKind =
    context?.activeReadProvider?.kind ?? kind ?? "local_hsd";

  const label =
    context?.activeReadProvider?.label ??
    providerLabel(resolvedKind, remoteLabel);

  const variant = context
    ? TONE_TO_VARIANT[providerTone(context)]
    : resolvedKind === "external_hnsfans"
      ? "warning"
      : "default";

  const readOnly =
    resolvedKind === "external_hnsfans" ||
    (context ? !context.writeAllowed : false);

  return (
    <Badge variant={variant} className={className}>
      {label}
      {readOnly ? " · read-only" : ""}
    </Badge>
  );
}
