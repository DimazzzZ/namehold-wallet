import { cn } from "../../lib/utils";

interface CardProps {
  title?: React.ReactNode;
  subtitle?: React.ReactNode;
  actions?: React.ReactNode;
  className?: string;
  bodyClassName?: string;
  padded?: boolean;
  children: React.ReactNode;
}

export function Card({
  title,
  subtitle,
  actions,
  className,
  bodyClassName,
  padded = true,
  children,
}: CardProps) {
  const hasHeader = title || subtitle || actions;
  return (
    <section
      className={cn(
        "bg-white rounded-lg border border-gray-200 shadow-sm",
        className,
      )}
    >
      {hasHeader && (
        <header className="flex items-start justify-between gap-3 px-4 py-3 border-b border-gray-100">
          <div className="min-w-0">
            {title && (
              <h3 className="text-sm font-semibold text-gray-900 truncate">
                {title}
              </h3>
            )}
            {subtitle && (
              <p className="text-xs text-gray-500 mt-0.5">{subtitle}</p>
            )}
          </div>
          {actions && <div className="flex items-center gap-2 shrink-0">{actions}</div>}
        </header>
      )}
      <div className={cn(padded && "p-4", bodyClassName)}>{children}</div>
    </section>
  );
}
