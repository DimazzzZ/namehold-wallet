import { useState } from "react";
import { Link, NavLink, Outlet } from "react-router-dom";
import { useActiveProfile, useWriteCapability } from "../queries/wallet";
import { Toast } from "./ui/Toast";
import { StatusStrip } from "./ui/StatusStrip";
import { UpdateBanner } from "./UpdateBanner";
import { PRIMARY_ROUTES } from "../lib/navigation";
import { cn } from "../lib/utils";
import { isTauri } from "../lib/runtime";
import { useAppHotkeys } from "../hooks/useAppHotkeys";
import { Cheatsheet } from "./Cheatsheet";

export function Layout() {
  const { data: profile } = useActiveProfile();
  const { data: writeCap } = useWriteCapability();

  const network = profile?.network ?? "no wallet";
  const canWrite = writeCap?.canWrite ?? false;

  const [cheatsheetOpen, setCheatsheetOpen] = useState(false);
  useAppHotkeys({ setCheatsheetOpen });

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
                canWrite
                  ? "bg-green-100 text-green-700"
                  : "bg-gray-100 text-gray-600",
              )}
            >
              {canWrite ? "CAN SEND" : "READ-ONLY"}
            </span>
          </div>
        </div>
        <nav className="flex-1 py-2">
          {PRIMARY_ROUTES.map((item) => (
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
          <div className="flex items-center justify-between">
            <div>
              <div className="text-[10px] text-gray-400 mt-0.5">v0.4.1</div>
            </div>
            <Link
              to="/about"
              className="text-gray-400 hover:text-gray-600 transition-colors"
              title="About"
            >
              ℹ️
            </Link>
          </div>
        </div>
      </aside>
      <main className="flex-1 flex flex-col overflow-hidden">
        <UpdateBanner />
        {!isTauri && (
          <div
            className="px-6 py-1.5 text-xs text-blue-900 bg-blue-100 border-b border-blue-200"
            data-testid="web-qa-banner"
          >
            🌐 <strong>Browser QA mode</strong> — mock backend active. No real wallet or
            node connected. UI and navigation work; data is simulated.
          </div>
        )}
        <div
          className="px-6 py-1.5 text-xs text-amber-900 bg-amber-100 border-b border-amber-200"
          data-testid="beta-banner"
        >
          ⚠️ <strong>Beta software</strong> — it can make mistakes. Always test with a
          single name or a small amount before transferring or sending everything.
        </div>
        <header className="flex items-center justify-end gap-4 px-6 py-2 border-b border-gray-200 bg-white">
          <StatusStrip />
        </header>
        <div className="flex-1 overflow-auto p-6">
          <Outlet />
        </div>
      </main>
      <Toast />
      <Cheatsheet open={cheatsheetOpen} onClose={() => setCheatsheetOpen(false)} />
    </div>
  );
}
