import type { ComponentProps } from "react";
import { Button } from "../ui/Button";
import { Tooltip } from "../ui/Tooltip";

type GatedButtonProps = ComponentProps<typeof Button> & {
  /** Why the button may not act, or null. Disables it and becomes its hint: never hidden. */
  refusal: string | null;
  /** The trigger wrapper's layout classes (see Tooltip's `className`). */
  wrapClassName?: string;
};

/**
 * A button the exchange's gate may refuse: disabled with the refusal as its
 * Tooltip (a disabled Button has `pointer-events: none`, so the hint sits on
 * the wrapper), and a `data-refusal` attribute carrying the same sentence.
 */
export function GatedButton({
  refusal,
  wrapClassName,
  disabled,
  children,
  ...props
}: GatedButtonProps) {
  return (
    <Tooltip content={refusal} className={wrapClassName}>
      <Button
        {...props}
        disabled={disabled || refusal !== null}
        data-refusal={refusal ?? undefined}
      >
        {children}
      </Button>
    </Tooltip>
  );
}
