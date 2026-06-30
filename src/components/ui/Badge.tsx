import { cn } from "../../lib/utils";

interface BadgeProps {
  children: React.ReactNode;
  variant?: "default" | "success" | "warning" | "error" | "info";
  className?: string;
  title?: string;
}

const variants = {
  default: "bg-gray-100 text-gray-700",
  success: "bg-green-100 text-green-800",
  warning: "bg-yellow-100 text-yellow-800",
  error: "bg-red-100 text-red-800",
  info: "bg-blue-100 text-blue-800",
};

export function Badge({ children, variant = "default", className, title }: BadgeProps) {
  return (
    <span
      title={title}
      className={cn(
        "inline-block px-2 py-0.5 rounded text-xs font-medium",
        variants[variant],
        className,
      )}
    >
      {children}
    </span>
  );
}
