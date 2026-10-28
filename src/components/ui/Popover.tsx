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
  /** Controlled mode: the caller owns the open state. Uncontrolled by default. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Which edge of the trigger the panel lines up with. */
  align?: "left" | "right";
  role?: "menu" | "dialog";
  "aria-label"?: string;
  "data-testid"?: string;
}

/**
 * A minimal anchored popover: a trigger with a floating panel positioned just
 * below it. The panel closes on outside click, Escape, or when a child calls
 * the injected `close()` (e.g. after a menu item fires). No floating-ui
 * dependency — these menus are short and anchored directly under the trigger.
 *
 * Close-on-outside-click mirrors `Dialog`'s dismiss intent but uses a ref +
 * document listener instead of a full-screen backdrop, so the rest of the UI
 * stays interactive while the menu is open. Escape is caught in the capture
 * phase and stopped, so a dialog underneath or the cheatsheet doesn't close
 * with it (the CommandPalette pattern).
 */
export function Popover({
  trigger,
  children,
  className,
  panelClassName,
  open: openProp,
  onOpenChange,
  align = "left",
  role = "menu",
  ...rest
}: PopoverProps) {
  const [openState, setOpenState] = useState(false);
  const controlled = openProp !== undefined;
  const open = controlled ? openProp : openState;
  const setOpen = (v: boolean) => {
    if (!controlled) setOpenState(v);
    onOpenChange?.(v);
  };
  const rootRef = useRef<HTMLDivElement>(null);
  // The listeners below must see the latest setter without re-subscribing.
  const setOpenRef = useRef(setOpen);
  setOpenRef.current = setOpen;

  const close = () => setOpen(false);
  const toggle = () => setOpen(!open);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpenRef.current(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setOpenRef.current(false);
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open]);

  return (
    <div ref={rootRef} className={cn("relative inline-block", className)}>
      {trigger({ open, toggle })}
      {open && (
        <div
          role={role}
          aria-label={rest["aria-label"]}
          data-testid={rest["data-testid"]}
          className={cn(
            "absolute top-full z-50 mt-1 min-w-40 rounded-md border border-gray-200 bg-white py-1 shadow-lg",
            align === "right" ? "right-0 left-auto" : "left-0",
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
export function PopoverItem({ onClick, disabled, children, className, ...rest }: PopoverItemProps) {
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
