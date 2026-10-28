import { useEffect, useState } from "react";
import { Link, NavLink, Outlet, useLocation } from "react-router-dom";
import { FlaskConical, Info, Keyboard } from "lucide-react";
import { useCurrentVersion } from "../queries/updates";
import { Toast } from "./ui/Toast";
import { UpdateBanner } from "./UpdateBanner";
import { PRIMARY_ROUTES } from "../lib/navigation";
import { cn } from "../lib/utils";
import { isBrowser } from "../lib/runtime";
import { useAppHotkeys } from "../hooks/useAppHotkeys";
import { useNavCounts } from "../hooks/useNavCounts";
import { useUiStore } from "../stores/ui";
import { Cheatsheet } from "./Cheatsheet";
import { CommandPalette } from "./CommandPalette";
import { WalletManager } from "./WalletManager";
import { Tooltip } from "./ui/Tooltip";
import { Icon } from "./ui/Icon";
import { WalletSwitcher } from "./shell/WalletSwitcher";
import { ReadinessPill } from "./shell/ReadinessPill";
import { NetworkStrip } from "./shell/NetworkStrip";

export const BETA_TEXT =
  "Beta software — it can make mistakes. Always test with a single name or a small amount before transferring or sending everything.";

function plural(n: number, one: string, many: string) {
  return `${n} ${n === 1 ? one : many}`;
}

export function Layout() {
  const { data: appVersion } = useCurrentVersion();
  const counts = useNavCounts();
  const walletsDialog = useUiStore((s) => s.walletsDialog);
  const closeWallets = useUiStore((s) => s.closeWallets);
  const setWalletMenuOpen = useUiStore((s) => s.setWalletMenuOpen);
  const setReadinessOpen = useUiStore((s) => s.setReadinessOpen);
  const location = useLocation();

  const [cheatsheetOpen, setCheatsheetOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  useAppHotkeys({ setCheatsheetOpen, setPaletteOpen });

  // Navigating anywhere closes the shell's popovers.
  useEffect(() => {
    setWalletMenuOpen(false);
    setReadinessOpen(false);
  }, [location.pathname, setWalletMenuOpen, setReadinessOpen]);

  const countFor = (key: string) =>
    key === "names" ? counts.names : key === "activity" ? counts.activity : 0;

  return (
    <div className="flex h-screen bg-gray-100">
      <aside className="w-56 bg-white border-r border-gray-200 flex flex-col shrink-0">
        <div className="px-4 py-3 border-b border-gray-200 flex flex-col gap-2.5">
          <div className="flex items-center justify-between">
            <h1 className="text-sm font-bold text-gray-900">Namehold</h1>
            <Tooltip content={BETA_TEXT}>
              <span
                tabIndex={0}
                data-testid="beta-badge"
                className="text-[10px] font-medium px-1.5 py-px rounded bg-yellow-100 text-yellow-800 cursor-help"
              >
                Beta
              </span>
            </Tooltip>
          </div>
          <WalletSwitcher />
        </div>
        <nav className="flex-1 py-2">
          {PRIMARY_ROUTES.map((item) => {
            const count = countFor(item.key);
            return (
              <Tooltip key={item.key} content={item.description} className="block">
                <NavLink
                  to={item.to}
                  end={item.to === "/"}
                  className={({ isActive }) =>
                    cn(
                      "flex items-center gap-2.5 px-4 py-2 text-sm text-gray-700 hover:bg-gray-50 border-r-2 border-transparent",
                      isActive && "bg-blue-50 text-blue-700 font-medium border-blue-700",
                    )
                  }
                >
                  <Icon icon={item.icon} />
                  <span>{item.label}</span>
                  {item.badge && (
                    <span
                      data-testid={`nav-badge-${item.key}`}
                      className="text-[10px] font-medium px-1.5 py-px rounded bg-blue-100 text-blue-800"
                    >
                      {item.badge}
                    </span>
                  )}
                  {count > 0 && (
                    <Tooltip
                      content={
                        item.key === "names"
                          ? `${plural(count, "name", "names")} ${count === 1 ? "needs" : "need"} you`
                          : `${count} to finish`
                      }
                      className="ml-auto inline-flex"
                    >
                      <span
                        data-testid={`nav-count-${item.key}`}
                        className="min-w-[18px] h-[18px] px-1.5 rounded-full bg-amber-100 text-amber-800 text-[11px] font-semibold inline-flex items-center justify-center tabular-nums"
                      >
                        {count}
                      </span>
                    </Tooltip>
                  )}
                </NavLink>
              </Tooltip>
            );
          })}
        </nav>
        <div className="px-4 py-2 border-t border-gray-200 flex items-center justify-between">
          <div className="text-[10px] text-gray-400">{appVersion ? `v${appVersion}` : ""}</div>
          <div className="flex items-center gap-1">
            <Tooltip content="Keyboard shortcuts (Shift+?)">
              <button
                type="button"
                onClick={() => setCheatsheetOpen(true)}
                aria-label="Keyboard shortcuts"
                className="p-1 rounded text-gray-400 hover:bg-gray-100 hover:text-gray-600"
              >
                <Icon icon={Keyboard} />
              </button>
            </Tooltip>
            <Tooltip content="About">
              <Link
                to="/about"
                aria-label="About"
                className="p-1 rounded text-gray-400 hover:bg-gray-100 hover:text-gray-600"
              >
                <Icon icon={Info} />
              </Link>
            </Tooltip>
          </div>
        </div>
      </aside>
      <main className="flex-1 flex flex-col overflow-hidden">
        <NetworkStrip />
        <UpdateBanner />
        {isBrowser() && (
          <div
            className="px-6 py-1.5 text-xs text-blue-900 bg-blue-100 border-b border-blue-200 flex items-center gap-1.5"
            data-testid="web-qa-banner"
          >
            <Icon icon={FlaskConical} size={14} />
            <span>
              <strong>Browser QA mode</strong> — mock backend active. No real wallet or node
              connected. UI and navigation work; data is simulated.
            </span>
          </div>
        )}
        <header className="flex items-center justify-end gap-3 px-6 py-2 border-b border-gray-200 bg-white min-h-[41px]">
          <ReadinessPill />
        </header>
        <div className="flex-1 overflow-auto p-6">
          <Outlet />
        </div>
      </main>
      <Toast />
      <Cheatsheet open={cheatsheetOpen} onClose={() => setCheatsheetOpen(false)} />
      <CommandPalette open={paletteOpen} onClose={() => setPaletteOpen(false)} />
      <WalletManager
        open={walletsDialog !== null}
        startInAddMode={walletsDialog === "add"}
        onClose={closeWallets}
      />
    </div>
  );
}
