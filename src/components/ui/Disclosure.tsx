import { useState, type ReactNode } from "react";
import { cn } from "../../lib/utils";

interface DisclosureProps {
  summary: ReactNode;
  children: ReactNode;
  defaultOpen?: boolean;
  className?: string;
  /** `data-testid` of the toggle button. */
  testId?: string;
}

/**
 * A minimal collapsible section: a chevron + summary button over a body.
 *
 * The body is ALWAYS mounted and hidden with CSS (`hidden`) when collapsed —
 * never conditionally unmounted. This keeps its content in the DOM (so tests
 * and screen readers can find it, and expanding is instant) while still hiding
 * it visually. `aria-expanded` reflects the open state.
 */
export function Disclosure({
  summary,
  children,
  defaultOpen = false,
  className,
  testId,
}: DisclosureProps) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className={className}>
      <button
        type="button"
        aria-expanded={open}
        data-testid={testId}
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-1 text-sm text-gray-600 hover:text-gray-900"
      >
        <span
          aria-hidden="true"
          className={cn("inline-block transition-transform", open && "rotate-90")}
        >
          ›
        </span>
        {summary}
      </button>
      <div className={open ? "mt-3" : "hidden"}>{children}</div>
    </div>
  );
}
