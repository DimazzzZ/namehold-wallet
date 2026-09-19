import { useRef, useState, type ReactNode } from "react";
import {
  useFloating,
  autoUpdate,
  offset,
  flip,
  shift,
  arrow,
  useHover,
  useFocus,
  useDismiss,
  useRole,
  useInteractions,
  FloatingPortal,
  FloatingArrow,
} from "@floating-ui/react";
import { cn } from "../../lib/utils";

/**
 * How long the pointer must rest on a trigger before the tooltip appears.
 *
 * This is the app's single hover debounce. Without it, sweeping the pointer
 * across a toolbar or a table row flashes a tooltip per element; native `title`
 * had the browser's own ~1s delay, and replacing it with an instant tooltip
 * traded one annoyance for a worse one. Closing stays immediate — a tooltip
 * that lingers after the pointer leaves reads as stuck.
 */
export const TOOLTIP_OPEN_DELAY_MS = 300;

interface TooltipProps {
  /**
   * Tooltip body. Kept short — one or two lines. When empty (`null`,
   * `undefined` or `""`) the children render bare, with no wrapper and no
   * hover handling, so callers can pass a conditional reason directly.
   */
  content: ReactNode;
  /** The trigger. Rendered inline; gets the hover/focus reference props. */
  children: ReactNode;
  /** Preferred side. floating-ui flips it near a scroll edge. */
  placement?: "top" | "bottom" | "left" | "right";
  /**
   * Dotted-underline affordance, for a word or figure inside running text
   * whose tooltip is the only hint that there is more to see. Leave it off
   * when the trigger is already an obvious control (a button, an icon, a
   * badge) — underlining those just adds noise.
   */
  hint?: boolean;
  /** Extra classes for the trigger wrapper. */
  className?: string;
  /** Override the hover debounce, in ms. `0` shows instantly. */
  openDelay?: number;
}

/**
 * Hover/focus tooltip built on @floating-ui/react. Flips and shifts to stay
 * inside a scrolling container (e.g. the activity table's `overflow:auto`), and
 * is keyboard- and screen-reader accessible via `useFocus` + `useRole`. Styled
 * to match the app's Popover card (white, subtle border, shadow, rounded).
 *
 * This replaces the native `title` attribute everywhere in the app. Two reasons
 * beyond styling: a `title` cannot be styled or positioned, and — the one that
 * actually bit us — an element with `pointer-events: none` (every disabled
 * `Button`) never shows one, so the hint was dead in the state it existed for.
 * The wrapper here keeps receiving pointer events whatever the child does.
 */
export function Tooltip({
  content,
  children,
  placement = "top",
  hint = false,
  className,
  openDelay = TOOLTIP_OPEN_DELAY_MS,
}: TooltipProps) {
  const [open, setOpen] = useState(false);
  const arrowRef = useRef<SVGSVGElement>(null);

  const { refs, floatingStyles, context } = useFloating({
    open,
    onOpenChange: setOpen,
    placement,
    whileElementsMounted: autoUpdate,
    middleware: [
      offset(6),
      flip({ padding: 8 }),
      shift({ padding: 8 }),
      arrow({ element: arrowRef }),
    ],
  });

  const hover = useHover(context, { move: false, delay: { open: openDelay, close: 0 } });
  const focus = useFocus(context);
  const dismiss = useDismiss(context);
  const role = useRole(context, { role: "tooltip" });
  const { getReferenceProps, getFloatingProps } = useInteractions([hover, focus, dismiss, role]);

  // Keeps `<Tooltip content={maybe}>` usable without the caller guarding first.
  // The wrapper is rendered either way: unmounting it as the content appears
  // and disappears would remount the child, dropping focus and caret position
  // in anything interactive — and a button that React replaces mid-interaction
  // is a stale node to anyone holding a reference to it.
  const hasContent = content != null && content !== "";

  return (
    <>
      <span
        ref={refs.setReference}
        {...getReferenceProps()}
        className={cn(
          hint
            ? "cursor-help underline decoration-dotted decoration-gray-400 underline-offset-2"
            : "inline-flex",
          className,
        )}
      >
        {children}
      </span>
      {open && hasContent && (
        <FloatingPortal>
          <div
            ref={refs.setFloating}
            style={floatingStyles}
            {...getFloatingProps()}
            className="z-50 max-w-xs rounded-md border border-gray-200 bg-white px-3 py-2 text-xs font-normal leading-snug text-gray-700 shadow-lg"
          >
            {content}
            <FloatingArrow
              ref={arrowRef}
              context={context}
              className="fill-white [&>path:first-of-type]:stroke-gray-200"
            />
          </div>
        </FloatingPortal>
      )}
    </>
  );
}
