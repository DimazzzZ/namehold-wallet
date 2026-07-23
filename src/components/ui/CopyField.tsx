import { useState } from "react";
import { writeText } from "../../lib/clipboard";
import { useUiStore } from "../../stores/ui";
import { Button } from "./Button";

interface CopyFieldProps {
  label?: React.ReactNode;
  /** The full string that is copied to the clipboard. */
  value: string;
  /**
   * Optional visible text shown INSTEAD of `value` (e.g. a middle-truncated
   * preview). Copy always uses the full `value`, never `display`.
   */
  display?: string;
  copyLabel?: string;
  /** Toast reads `${toastLabel} copied` on success. */
  toastLabel?: string;
  valueTestId?: string;
  copyTestId?: string;
  testId?: string;
}

/**
 * Read-only mono string + a Copy button with "Copied!" feedback and a toast.
 * Centralizes the copy idiom (writeText + copied state + toast + 2s reset) that
 * was previously inlined at every call site.
 */
export function CopyField({
  label,
  value,
  display,
  copyLabel = "Copy",
  toastLabel = "Copied",
  valueTestId,
  copyTestId,
  testId,
}: CopyFieldProps) {
  const [copied, setCopied] = useState(false);
  const showToast = useUiStore((s) => s.showToast);

  const handleCopy = async () => {
    if (!value) return;
    await writeText(value);
    setCopied(true);
    showToast(`${toastLabel} copied`, "success");
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div data-testid={testId}>
      {label && <div className="text-sm text-gray-500 mb-1">{label}</div>}
      <div className="flex items-center gap-2">
        <code
          data-testid={valueTestId}
          className="font-mono text-sm break-all flex-1 bg-gray-50 p-2 rounded"
        >
          {display ?? value}
        </code>
        <Button
          variant="secondary"
          size="sm"
          onClick={handleCopy}
          disabled={!value}
          data-testid={copyTestId}
          className="shrink-0"
        >
          {copied ? "Copied!" : copyLabel}
        </Button>
      </div>
    </div>
  );
}
