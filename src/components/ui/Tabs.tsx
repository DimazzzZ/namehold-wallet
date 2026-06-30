import { cn } from "../../lib/utils";
import type { WorkspaceTab } from "../../types";

interface TabsProps<T extends string> {
  tabs: WorkspaceTab<T>[];
  active: T;
  onChange: (key: T) => void;
  className?: string;
}

export function Tabs<T extends string>({
  tabs,
  active,
  onChange,
  className,
}: TabsProps<T>) {
  return (
    <div
      role="tablist"
      className={cn("inline-flex flex-wrap items-center gap-1.5", className)}
    >
      {tabs.map((tab) => {
        const isActive = tab.key === active;
        return (
          <button
            key={tab.key}
            role="tab"
            aria-selected={isActive}
            title={tab.description}
            onClick={() => onChange(tab.key)}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-md border px-3.5 py-1.5 text-sm font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 focus-visible:ring-offset-1",
              isActive
                ? "bg-blue-600 text-white border-blue-600 shadow-sm"
                : "bg-gray-100 text-gray-900 border-gray-300 hover:bg-gray-200",
            )}
          >
            {tab.label}
            {tab.badge !== undefined && (
              <span
                className={cn(
                  "rounded-full px-1.5 text-[10px] font-semibold",
                  isActive ? "bg-white/20 text-white" : "bg-gray-200 text-gray-600",
                )}
              >
                {tab.badge}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
