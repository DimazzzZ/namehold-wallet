import { X } from "lucide-react";
import { Icon } from "./Icon";
import { useEffect, useRef } from "react";

interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: React.ReactNode;
  children: React.ReactNode;
  /** Override the default max-width (`max-w-lg`). Pass a Tailwind max-w class. */
  className?: string;
}

interface OpenDialog {
  panel: React.RefObject<HTMLDivElement | null>;
  onClose: React.RefObject<() => void>;
}

// Every open dialog, in the order it opened. One document listener answers
// Escape for all of them, so which dialog closes never depends on the order
// listeners were registered in: parents pass a fresh inline `onClose`, and a
// dialog that re-subscribed after the one opened inside it would otherwise
// run second and, after a browser that flushes the inner close between
// listeners (WebKit), find no nested panel and close as well.
const openDialogs: OpenDialog[] = [];

// The innermost open dialog: the latest one whose panel holds no other open
// dialog's panel. This relies on a nested dialog rendering inside the outer
// panel: Dialog uses no portal.
function innermost(): OpenDialog | undefined {
  return [...openDialogs].reverse().find((dialog) => {
    const own = dialog.panel.current;
    return (
      own &&
      !openDialogs.some((d) => d !== dialog && d.panel.current && own.contains(d.panel.current))
    );
  });
}

function onEscape(e: KeyboardEvent) {
  if (e.key === "Escape") innermost()?.onClose.current();
}

export function Dialog({ open, onClose, title, children, className }: DialogProps) {
  const panel = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    if (!open) return;
    const entry: OpenDialog = { panel, onClose: close };
    openDialogs.push(entry);
    if (openDialogs.length === 1) document.addEventListener("keydown", onEscape);
    return () => {
      const i = openDialogs.indexOf(entry);
      if (i >= 0) openDialogs.splice(i, 1);
      if (openDialogs.length === 0) document.removeEventListener("keydown", onEscape);
    };
  }, [open]);

  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="fixed inset-0 bg-black/50" onClick={onClose} />
      {/* `role="dialog"` and nothing stronger: `aria-modal` would claim the
          rest of the page is inert, and this component does not trap focus. */}
      <div
        ref={panel}
        data-dialog-panel
        role="dialog"
        className={`relative bg-white rounded-lg shadow-xl w-full mx-4 max-h-[90vh] overflow-auto ${className ?? "max-w-lg"}`}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200">
          <h2 className="text-lg font-semibold">{title}</h2>
          <button
            onClick={onClose}
            aria-label="Close"
            className="text-gray-400 hover:text-gray-600 cursor-pointer"
          >
            <Icon icon={X} size={18} />
          </button>
        </div>
        <div className="p-4">{children}</div>
      </div>
    </div>
  );
}
