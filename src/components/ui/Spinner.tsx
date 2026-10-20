import { cn } from "../../lib/utils";

/** A small spinner in the text's colour; still for those who ask for reduced motion. */
export function Spinner({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-block h-3 w-3 shrink-0 rounded-full border-2 border-current border-t-transparent motion-safe:animate-spin",
        className,
      )}
    />
  );
}
