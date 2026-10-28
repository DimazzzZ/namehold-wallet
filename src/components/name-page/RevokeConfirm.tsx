import { useState } from "react";
import { Button } from "../ui/Button";
import { displayName } from "../../lib/idn";

/**
 * Type-to-confirm for Revoke: the name, decoded or raw, with or without the
 * leading dot. This is the only way to revoke.
 */
export function RevokeConfirm({
  name,
  allowed,
  busy,
  onConfirm,
  onCancel,
}: {
  name: string;
  allowed: boolean;
  busy: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const decoded = displayName(name);
  const [typed, setTyped] = useState("");
  const t = typed.trim().replace(/^\./, "");
  const matches = t === decoded || t === name;

  return (
    <div className="border-t border-red-100 pt-2.5 flex flex-col gap-2" data-testid="revoke-box">
      <label className="text-[13px] text-gray-700" htmlFor="revoke-confirm-input">
        Type <b className="font-mono">{decoded}</b> to confirm
      </label>
      <input
        id="revoke-confirm-input"
        className="w-full rounded border border-red-300 px-2 py-1 font-mono text-sm"
        value={typed}
        placeholder={decoded}
        autoFocus
        onChange={(e) => setTyped(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            onCancel();
          }
        }}
        data-testid="revoke-confirm-input"
      />
      <div className="flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          size="sm"
          variant="danger"
          disabled={!matches || !allowed || busy}
          onClick={onConfirm}
          data-testid="revoke-confirm"
        >
          {busy ? "Revoking…" : `Revoke .${decoded}`}
        </Button>
      </div>
    </div>
  );
}
