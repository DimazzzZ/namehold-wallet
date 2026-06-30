import { Button } from "./Button";
import type { PageAction } from "../../types";

interface EmptyStateProps {
  title: string;
  description?: React.ReactNode;
  actions?: PageAction[];
  icon?: React.ReactNode;
}

export function EmptyState({ title, description, actions, icon }: EmptyStateProps) {
  return (
    <div className="bg-white rounded-lg border border-dashed border-gray-300 p-8 text-center">
      {icon && <div className="mb-3 flex justify-center text-gray-400">{icon}</div>}
      <div className="text-sm font-semibold text-gray-700">{title}</div>
      {description && (
        <div className="text-sm text-gray-400 mt-1 max-w-md mx-auto">{description}</div>
      )}
      {actions && actions.length > 0 && (
        <div className="mt-4 flex items-center justify-center gap-2">
          {actions.map((action) => (
            <Button
              key={action.label}
              variant={action.variant ?? "primary"}
              disabled={action.disabled || action.loading}
              onClick={action.onClick}
            >
              {action.loading ? "Working..." : action.label}
            </Button>
          ))}
        </div>
      )}
    </div>
  );
}
