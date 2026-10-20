import { cn } from "../../lib/utils";
import { Spinner } from "./Spinner";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  /** `fixit`: a primary slot whose action enables a prerequisite ("Turn on buying"). */
  variant?: "primary" | "secondary" | "danger" | "ghost" | "fixit";
  size?: "sm" | "md";
  loading?: boolean;
}

export function Button({
  variant = "secondary",
  size = "md",
  loading = false,
  disabled,
  className,
  children,
  ...props
}: ButtonProps) {
  const base =
    "inline-flex items-center justify-center gap-1.5 rounded font-medium cursor-pointer transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 focus-visible:ring-offset-1 disabled:opacity-50 disabled:pointer-events-none";
  const variants = {
    primary: "bg-blue-600 text-white hover:bg-blue-700 border border-transparent",
    secondary: "bg-white text-gray-900 hover:bg-gray-50 border border-gray-300",
    danger: "bg-red-600 text-white hover:bg-red-700 border border-transparent",
    ghost: "text-gray-600 hover:bg-gray-100 border border-transparent",
    fixit: "bg-white text-blue-700 hover:bg-blue-50 border border-blue-300",
  };
  const sizes = {
    sm: "px-2.5 py-1 text-xs",
    md: "px-3.5 py-1.5 text-sm",
  };
  return (
    <button
      className={cn(base, variants[variant], sizes[size], className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...props}
    >
      {loading && <Spinner />}
      {children}
    </button>
  );
}
