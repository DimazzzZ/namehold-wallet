import { NavLink, Outlet } from "react-router-dom";
import { useSettingsStore } from "../stores/settings";
import { useNodeStatus } from "../queries/node";
import { Toast } from "./ui/Toast";
import { cn } from "../lib/utils";

const NAV_ITEMS = [
  { to: "/", label: "Dashboard" },
  { to: "/inventory", label: "TLD Inventory" },
  { to: "/batches", label: "Batches" },
  { to: "/wallet", label: "Wallet" },
  { to: "/wallets", label: "Wallet Manager" },
  { to: "/sync", label: "Sync" },
  { to: "/renewals", label: "Renewals" },
  { to: "/dns", label: "DNS Records" },
  { to: "/node", label: "Node Control" },
  { to: "/settings", label: "Settings" },
];

export function Layout() {
  const settings = useSettingsStore((s) => s.settings);
  const { data: nodeStatus } = useNodeStatus();
  const network = settings?.hsd_network || "unknown";
  const writeMode = settings?.write_mode === "true";
  const currentWallet = settings?.hsd_wallet_id || "";

  const nodeRunning = nodeStatus?.running ?? false;
  const walletConnected = nodeStatus?.wallet_connected ?? false;

  const connectionLabel = !nodeRunning
    ? "Node offline"
    : !currentWallet
      ? "No wallet"
      : walletConnected
        ? "Connected"
        : "Wallet error";

  const connectionColor = !nodeRunning
    ? "bg-red-500"
    : !currentWallet
      ? "bg-yellow-500"
      : walletConnected
        ? "bg-green-500"
        : "bg-yellow-500";

  return (
    <div className="flex h-screen bg-gray-100">
      <aside className="w-56 bg-white border-r border-gray-200 flex flex-col">
        <div className="px-4 py-3 border-b border-gray-200">
          <h1 className="text-sm font-bold text-gray-900">Namehold</h1>
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
        <div className="px-4 py-2 border-t border-gray-200">
          <div className="flex items-center gap-2">
            <div className={cn("w-2 h-2 rounded-full", connectionColor)} />
            <span className="text-[10px] text-gray-500">{connectionLabel}</span>
          </div>
          <div className="text-[10px] text-gray-400 mt-1">
            {currentWallet || "No wallet selected"}
          </div>
          <div className="text-[10px] text-gray-400 mt-0.5">
            v0.1.0
          </div>
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
