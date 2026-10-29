import { Icon } from "./Icon";
import { Link } from "react-router-dom";
import { Button } from "./Button";
import type { PageAction } from "../../types";

interface PageHeaderProps {
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  badges?: React.ReactNode;
  actions?: PageAction[];
  /** Free-form controls on the right, for actions that need more than a label (tooltips, dividers). */
  aside?: React.ReactNode;
  children?: React.ReactNode;
}

export function PageHeader({ title, subtitle, badges, actions, aside, children }: PageHeaderProps) {
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
      {aside && <div className="flex items-center gap-2 shrink-0">{aside}</div>}
      {actions && actions.length > 0 && (
        <div className="flex items-center gap-2 shrink-0">
          {actions.map((action) => {
            const label = action.loading ? "Working..." : action.label;
            const content = action.icon ? (
              <span className="inline-flex items-center gap-1.5">
                <Icon icon={action.icon} size={14} />
                {label}
              </span>
            ) : (
              label
            );
            if (action.to && !action.disabled && !action.loading) {
              return (
                <Link key={action.label} to={action.to}>
                  <Button variant={action.variant ?? "secondary"} data-testid={action.testId}>
                    {content}
                  </Button>
                </Link>
              );
            }
            return (
              <Button
                key={action.label}
                variant={action.variant ?? "secondary"}
                data-testid={action.testId}
                disabled={action.disabled}
                loading={action.loading}
                onClick={action.onClick}
              >
                {action.icon ? (
                  <span className="inline-flex items-center gap-1.5">
                    <Icon icon={action.icon} size={14} />
                    {action.label}
                  </span>
                ) : (
                  action.label
                )}
              </Button>
            );
          })}
        </div>
      )}
    </div>
  );
}
