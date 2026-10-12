import { useEffect, useRef } from "react";

interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: React.ReactNode;
  children: React.ReactNode;
  /** Override the default max-width (`max-w-lg`). Pass a Tailwind max-w class. */
  className?: string;
}

export function Dialog({ open, onClose, title, children, className }: DialogProps) {
  const panel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      // Every open dialog listens on the document. A dialog opened inside
      // this one (the Lock dialog in a name's actions) takes Escape alone:
      // closing this one too would unmount it mid-flow. This relies on a
      // nested dialog rendering inside this panel: Dialog uses no portal.
      if (e.key === "Escape" && !panel.current?.querySelector("[data-dialog-panel]")) {
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

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
        <div className="flex items-center justify-between px-4 py-3 border-b">
          <h2 className="text-lg font-semibold">{title}</h2>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-gray-600 text-xl leading-none cursor-pointer"
          >
            &times;
          </button>
        </div>
        <div className="p-4">{children}</div>
      </div>
    </div>
  );
}
