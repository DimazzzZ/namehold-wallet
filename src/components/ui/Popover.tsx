import { useEffect, useRef, useState, type ReactNode } from "react";
import { cn } from "../../lib/utils";

interface PopoverProps {
  /** The always-visible trigger. Clicking it toggles the panel. */
  trigger: (opts: { open: boolean; toggle: () => void }) => ReactNode;
  /** Panel body. Receives `close` so menu items can dismiss after firing. */
  children: (opts: { close: () => void }) => ReactNode;
  className?: string;
  /** Extra classes for the floating panel. */
  panelClassName?: string;
}

/**
 * A minimal anchored popover: a trigger with a floating panel positioned just
 * below it. The panel closes on outside click, Escape, or when a child calls
 * the injected `close()` (e.g. after a menu item fires). No floating-ui
 * dependency — these menus are short and anchored directly under the trigger.
 *
 * Close-on-outside-click mirrors `Dialog`'s dismiss intent but uses a ref +
 * document listener instead of a full-screen backdrop, so the rest of the UI
 * stays interactive while the menu is open.
 */
export function Popover({ trigger, children, className, panelClassName }: PopoverProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  const close = () => setOpen(false);
  const toggle = () => setOpen((v) => !v);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div ref={rootRef} className={cn("relative inline-block", className)}>
      {trigger({ open, toggle })}
      {open && (
        <div
          role="menu"
          className={cn(
            "absolute left-0 top-full z-50 mt-1 min-w-40 rounded-md border border-gray-200 bg-white py-1 shadow-lg",
            panelClassName,
          )}
        >
          {children({ close })}
        </div>
      )}
    </div>
  );
}

interface PopoverItemProps {
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
  className?: string;
  "data-testid"?: string;
}

/** A single clickable row inside a `Popover` panel. */
export function PopoverItem({
  onClick,
  disabled,
  children,
  className,
  ...rest
}: PopoverItemProps) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={disabled}
      onClick={onClick}
      data-testid={rest["data-testid"]}
      className={cn(
        "block w-full px-3 py-1.5 text-left text-xs text-gray-700 hover:bg-gray-100 disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
    >
      {children}
    </button>
  );
}
