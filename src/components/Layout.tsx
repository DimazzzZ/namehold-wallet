import { NavLink, Outlet } from "react-router-dom";
import { useSettingsStore } from "../stores/settings";
import { Toast } from "./ui/Toast";
import { cn } from "../lib/utils";

const NAV_ITEMS = [
  { to: "/", label: "Dashboard", icon: "~" },
  { to: "/inventory", label: "TLD Inventory", icon: "~" },
  { to: "/batches", label: "Batches", icon: "~" },
  { to: "/wallet", label: "Wallet", icon: "~" },
  { to: "/sync", label: "Sync", icon: "~" },
  { to: "/renewals", label: "Renewals", icon: "~" },
  { to: "/dns", label: "DNS Records", icon: "~" },
  { to: "/settings", label: "Settings", icon: "~" },
];

export function Layout() {
  const settings = useSettingsStore((s) => s.settings);
  const network = settings?.hsd_network || "unknown";
  const writeMode = settings?.write_mode === "true";

  return (
    <div className="flex h-screen bg-gray-100">
      <aside className="w-56 bg-white border-r border-gray-200 flex flex-col">
        <div className="px-4 py-3 border-b border-gray-200">
          <h1 className="text-sm font-bold text-gray-900">HNS Portfolio</h1>
          <div className="flex gap-2 mt-1">
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
        </div>
        <nav className="flex-1 py-2">
          {NAV_ITEMS.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.to === "/"}
              className={({ isActive }) =>
                cn(
                  "block px-4 py-2 text-sm text-gray-700 hover:bg-gray-100",
                  isActive && "bg-blue-50 text-blue-700 font-medium border-r-2 border-blue-700",
                )
              }
            >
              {item.label}
            </NavLink>
          ))}
        </nav>
        <div className="px-4 py-2 border-t border-gray-200 text-[10px] text-gray-400">
          v0.1.0
        </div>
      </aside>
      <main className="flex-1 overflow-auto">
        <div className="p-6">
          <Outlet />
        </div>
      </main>
      <Toast />
    </div>
  );
}
