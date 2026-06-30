import { NavLink, Outlet } from "react-router-dom";
import { useSettingsStore } from "../stores/settings";
import { useReadContext } from "../queries/read";
import { Toast } from "./ui/Toast";
import { StatusStrip } from "./ui/StatusStrip";
import { PRIMARY_ROUTES } from "../lib/navigation";
import { canWrite, providerStatusValue } from "../lib/providerMode";
import { cn } from "../lib/utils";

export function Layout() {
  const settings = useSettingsStore((s) => s.settings);
  const { data: readContext } = useReadContext();
  const network = settings?.hsd_network || "unknown";
  const currentWallet = settings?.hsd_wallet_id || "";
  const advancedMode = settings?.advanced_mode === "true";

  // The write badge reflects the active provider's real write capability when
  // the read context is loaded, falling back to the configured write_mode
  // setting before the context resolves.
  const writeMode = readContext
    ? canWrite(readContext)
    : settings?.write_mode === "true";
  const providerStatus = readContext ? providerStatusValue(readContext) : null;

  return (
    <div className="flex h-screen bg-gray-100">
      <aside className="w-56 bg-white border-r border-gray-200 flex flex-col">
        <div className="px-4 py-3 border-b border-gray-200">
          <h1 className="text-sm font-bold text-gray-900">Namehold</h1>
          <div className="flex flex-wrap gap-2 mt-1">
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-gray-100 text-gray-600">
              {network}
            </span>
            <span
              className={cn(
                "text-[10px] px-1.5 py-0.5 rounded",
                writeMode
                  ? "bg-red-100 text-red-700"
                  : "bg-green-100 text-green-700",
              )}
            >
              {writeMode ? "WRITE" : "READ-ONLY"}
            </span>
          </div>
          {providerStatus && (
            <div className="mt-1 text-[10px] text-gray-400">
              {readContext?.activeReadProvider?.label ?? "Provider"}: {providerStatus}
            </div>
          )}
        </div>
        <nav className="flex-1 py-2">
          {PRIMARY_ROUTES.filter((item) => !item.advanced || advancedMode).map((item) => (
            <NavLink
              key={item.key}
              to={item.to}
              end={item.to === "/"}
              title={item.description}
              className={({ isActive }) =>
                cn(
                  "block px-4 py-2 text-sm text-gray-700 hover:bg-gray-100",
                  isActive &&
                    "bg-blue-50 text-blue-700 font-medium border-r-2 border-blue-700",
                )
              }
            >
              {item.label}
            </NavLink>
          ))}
        </nav>
        <div className="px-4 py-2 border-t border-gray-200">
          <div className="text-[10px] text-gray-400">
            {currentWallet || "No wallet selected"}
          </div>
          <div className="text-[10px] text-gray-400 mt-0.5">v0.2.0</div>
        </div>
      </aside>
      <main className="flex-1 flex flex-col overflow-hidden">
        <header className="flex items-center justify-end gap-4 px-6 py-2 border-b border-gray-200 bg-white">
          <StatusStrip />
        </header>
        <div className="flex-1 overflow-auto p-6">
          <Outlet />
        </div>
      </main>
      <Toast />
    </div>
  );
}
