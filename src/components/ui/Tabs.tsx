import { cn } from "../../lib/utils";
import { Tooltip } from "./Tooltip";
import type { WorkspaceTab } from "../../types";

interface TabsProps<T extends string> {
  tabs: WorkspaceTab<T>[];
  active: T;
  onChange: (key: T) => void;
  className?: string;
}

/**
 * Underlined tabs: the active one blue-600 text over a 2px blue line, never a
 * filled button, so the page's one primary action stays the only one.
 */
export function Tabs<T extends string>({ tabs, active, onChange, className }: TabsProps<T>) {
  return (
    <div
      role="tablist"
      className={cn("flex flex-wrap items-end gap-5 border-b border-gray-200", className)}
    >
      {tabs.map((tab) => {
        const isActive = tab.key === active;
        return (
          <Tooltip key={tab.key} content={tab.description}>
            <button
              role="tab"
              aria-selected={isActive}
              onClick={() => onChange(tab.key)}
              className={cn(
                "-mb-px inline-flex items-center gap-1.5 border-b-2 px-0.5 pb-2 pt-1 text-sm font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 focus-visible:ring-offset-1",
                isActive
                  ? "border-blue-600 text-blue-600"
                  : "border-transparent text-gray-600 hover:border-gray-300 hover:text-gray-900",
              )}
            >
              {tab.label}
              {tab.badge !== undefined && (
                <span
                  className={cn(
                    "rounded-full px-1.5 text-[10px] font-semibold",
                    isActive ? "bg-blue-50 text-blue-700" : "bg-gray-100 text-gray-600",
                  )}
                >
                  {tab.badge}
                </span>
              )}
              {tab.hint && (
                <Tooltip content={tab.hint}>
                  <span
                    className="h-1.5 w-1.5 rounded-full bg-gray-300"
                    aria-label={tab.hint}
                    data-testid={`tab-hint-${tab.key}`}
                  />
                </Tooltip>
              )}
            </button>
          </Tooltip>
        );
      })}
    </div>
  );
}
