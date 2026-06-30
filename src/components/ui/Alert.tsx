import { cn } from "../../lib/utils";
import type { StatusTone } from "../../types";

interface AlertProps {
  tone?: StatusTone;
  title?: React.ReactNode;
  className?: string;
  children?: React.ReactNode;
}

const TONE_STYLES: Record<StatusTone, string> = {
  default: "bg-gray-50 border-gray-200 text-gray-700",
  info: "bg-blue-50 border-blue-200 text-blue-800",
  success: "bg-green-50 border-green-200 text-green-800",
  warning: "bg-yellow-50 border-yellow-200 text-yellow-800",
  error: "bg-red-50 border-red-200 text-red-800",
};

export function Alert({ tone = "info", title, className, children }: AlertProps) {
  return (
    <div
      role={tone === "error" ? "alert" : "status"}
      className={cn(
        "rounded-md border px-3 py-2 text-sm",
        TONE_STYLES[tone],
        className,
      )}
    >
      {title && <div className="font-semibold mb-0.5">{title}</div>}
      {children && <div className="text-xs leading-relaxed">{children}</div>}
    </div>
  );
}
