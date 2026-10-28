import type { LucideIcon, LucideProps } from "lucide-react";
import { cn } from "../../lib/utils";

type IconProps = Omit<LucideProps, "ref" | "size"> & {
  icon: LucideIcon;
  size?: 12 | 13 | 14 | 15 | 16 | 18;
  /** Accessible name when the icon stands alone; otherwise it is hidden. */
  label?: string;
};

/**
 * Every Lucide icon in the app goes through here: one stroke width, colour
 * from `currentColor`. Icon-only buttons name the button, not the icon.
 */
export function Icon({
  icon: Glyph,
  size = 16,
  strokeWidth = 1.75,
  label,
  className,
  ...rest
}: IconProps) {
  return (
    <Glyph
      size={size}
      strokeWidth={strokeWidth}
      className={cn("shrink-0", className)}
      aria-hidden={label ? undefined : true}
      aria-label={label}
      role={label ? "img" : undefined}
      {...rest}
    />
  );
}
