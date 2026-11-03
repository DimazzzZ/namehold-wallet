import { TriangleAlert } from "lucide-react";
import { confusable } from "../lib/confusable";
import { Icon } from "./ui/Icon";

/** Says so when a name is written to read as another one. Renders nothing otherwise. */
export function ConfusableNameNote({ name, className = "" }: { name: string; className?: string }) {
  const found = confusable(name);
  if (!found) return null;
  return (
    <p
      className={`flex items-start gap-1.5 text-xs text-amber-800 bg-amber-50 border border-amber-200 rounded-md px-2.5 py-1.5 ${className}`}
      role="note"
      data-testid="confusable-name-note"
    >
      <Icon icon={TriangleAlert} size={14} className="mt-px shrink-0" />
      <span>
        {found.readsAs ? (
          <>
            This reads like <span className="font-mono">.{found.readsAs}</span>, but it is a
            different name written with look-alike letters{found.hidden && " and hidden characters"}
            :
          </>
        ) : (
          "This name holds characters that do not show:"
        )}{" "}
        <span className="font-mono break-all">.{name}</span>. Check it before you pay for it or send
        to it.
      </span>
    </p>
  );
}
