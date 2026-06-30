import { Link } from "react-router-dom";
import { Button } from "./Button";
import type { PageAction } from "../../types";

interface PageHeaderProps {
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  badges?: React.ReactNode;
  actions?: PageAction[];
  children?: React.ReactNode;
}

export function PageHeader({
  title,
  subtitle,
  badges,
  actions,
  children,
}: PageHeaderProps) {
  return (
    <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between mb-5">
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <h2 className="text-xl font-bold text-gray-900 truncate">{title}</h2>
          {badges}
        </div>
        {subtitle && <p className="text-sm text-gray-500 mt-1">{subtitle}</p>}
        {children}
      </div>
      {actions && actions.length > 0 && (
        <div className="flex items-center gap-2 shrink-0">
          {actions.map((action) => {
            const content = action.loading ? "Working..." : action.label;
            if (action.to && !action.disabled && !action.loading) {
              return (
                <Link key={action.label} to={action.to}>
                  <Button variant={action.variant ?? "secondary"}>{content}</Button>
                </Link>
              );
            }
            return (
              <Button
                key={action.label}
                variant={action.variant ?? "secondary"}
                disabled={action.disabled}
                loading={action.loading}
                onClick={action.onClick}
              >
                {action.label}
              </Button>
            );
          })}
        </div>
      )}
    </div>
  );
}
