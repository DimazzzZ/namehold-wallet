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

interface TooltipProps {
  /** Tooltip body. Kept short — one or two lines. */
  content: ReactNode;
  /** The trigger. Rendered inline; gets the hover/focus reference props. */
  children: ReactNode;
  /** Preferred side. floating-ui flips it near a scroll edge. */
  placement?: "top" | "bottom" | "left" | "right";
}

/**
 * Hover/focus tooltip built on @floating-ui/react. Shows instantly (no bounce
 * delay), flips and shifts to stay inside a scrolling container (e.g. the
 * activity table's `overflow:auto`), and is keyboard- and screen-reader
 * accessible via `useFocus` + `useRole("tooltip")`. Styled to match the app's
 * Popover card (white, subtle border, shadow, rounded).
 */
export function Tooltip({ content, children, placement = "top" }: TooltipProps) {
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

  const hover = useHover(context, { move: false });
  const focus = useFocus(context);
  const dismiss = useDismiss(context);
  const role = useRole(context, { role: "tooltip" });
  const { getReferenceProps, getFloatingProps } = useInteractions([hover, focus, dismiss, role]);

  return (
    <>
      <span
        ref={refs.setReference}
        {...getReferenceProps()}
        className="cursor-help underline decoration-dotted decoration-gray-400 underline-offset-2"
      >
        {children}
      </span>
      {open && (
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
