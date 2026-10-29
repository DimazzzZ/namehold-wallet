import { Check, ChevronsUpDown, Lock, LockOpen, Plus, Settings2 } from "lucide-react";
import { useSignerToggle } from "../../hooks/useSignerToggle";
import { mapError } from "../../lib/errors";
import { netTheme } from "../../lib/networkTheme";
import { cn } from "../../lib/utils";
import { walletKindLabel } from "../../lib/walletKind";
import { useActiveProfile, useSetActiveProfile, useWalletProfiles } from "../../queries/wallet";
import { useUiStore } from "../../stores/ui";
import type { WalletProfileSummary } from "../../types";
import { Icon } from "../ui/Icon";
import { Popover } from "../ui/Popover";
import { Tooltip } from "../ui/Tooltip";

/** "Hot · ●Mainnet": how the wallet signs, then its network in its own colour. */
function KindAndNetwork({ p }: { p: WalletProfileSummary }) {
  const t = netTheme(p.network);
  return (
    <span className="text-[11px] text-gray-500 truncate flex items-center gap-1">
      {walletKindLabel(p)} ·
      <span className={cn("size-1.5 rounded-full shrink-0", t.dot)} aria-hidden />
      <span className={cn("font-semibold", t.text)}>{t.label}</span>
    </span>
  );
}

/** The sidebar's wallet card (opens the wallet menu) and the signer lock beside it. */
export function WalletSwitcher() {
  const { data: profiles = [] } = useWalletProfiles();
  const { data: profile } = useActiveProfile();
  const setActive = useSetActiveProfile();
  const signer = useSignerToggle();
  const openWallets = useUiStore((s) => s.openWallets);
  const showToast = useUiStore((s) => s.showToast);
  const menuOpen = useUiStore((s) => s.walletMenuOpen);
  const setMenuOpen = useUiStore((s) => s.setWalletMenuOpen);
  const theme = netTheme(profile?.network ?? "mainnet");

  const pick = (p: WalletProfileSummary, close: () => void) => {
    close();
    if (p.id === profile?.id) return;
    setActive.mutate(p.id, { onError: (e) => showToast(mapError(e), "error") });
  };

  const lockTip = signer.unlocked
    ? "Lock the signer. It also locks by itself after the session timeout."
    : signer.hasPassphrase
      ? "Unlock the signer with your passphrase (in a secure window)"
      : "Unlock the signer — no passphrase set, one click";

  return (
    <div className="flex gap-1.5">
      <Popover
        open={menuOpen}
        onOpenChange={setMenuOpen}
        className="flex-1 min-w-0"
        panelClassName="w-62 p-1 rounded-lg shadow-lg"
        aria-label="Wallets"
        trigger={({ open, toggle }) => (
          <Tooltip content="Switch wallet" className="flex w-full">
            <button
              type="button"
              onClick={toggle}
              aria-haspopup="menu"
              aria-expanded={open}
              data-testid="wallet-switcher"
              className={cn(
                "w-full min-w-0 rounded-md border px-2 py-1.5 flex items-center gap-2 text-left hover:brightness-[.97]",
                theme.card,
              )}
            >
              <span className="flex-1 min-w-0 flex flex-col">
                <span className="text-[13px] font-semibold truncate text-gray-900">
                  {profile?.label ?? "No wallet"}
                </span>
                {profile ? (
                  <KindAndNetwork p={profile} />
                ) : (
                  <span className="text-[11px] text-gray-500 truncate">Create or import one</span>
                )}
              </span>
              <Icon icon={ChevronsUpDown} size={14} className="text-gray-400" />
            </button>
          </Tooltip>
        )}
      >
        {({ close }) => (
          <>
            {profiles.length === 0 ? (
              <div className="px-2.5 py-2 text-[13px] text-gray-500">No wallets yet.</div>
            ) : (
              profiles.map((p) => {
                const active = p.id === profile?.id;
                return (
                  <button
                    key={p.id}
                    type="button"
                    role="menuitem"
                    onClick={() => pick(p, close)}
                    data-testid="wallet-menu-item"
                    className={cn(
                      "w-full flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left hover:bg-gray-50",
                      active && "bg-gray-50",
                    )}
                  >
                    <span className="flex-1 min-w-0 flex flex-col">
                      <span
                        className={cn(
                          "text-[13px] truncate text-gray-900",
                          active ? "font-semibold" : "font-medium",
                        )}
                      >
                        {p.label}
                      </span>
                      <KindAndNetwork p={p} />
                    </span>
                    {active && <Icon icon={Check} size={14} className="text-blue-600" />}
                  </button>
                );
              })
            )}
            <div className="my-1 border-t border-gray-100" />
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                close();
                openWallets("add");
              }}
              className="w-full flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] text-gray-700 hover:bg-gray-50"
            >
              <Icon icon={Plus} size={14} /> Add wallet
            </button>
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                close();
                openWallets("list");
              }}
              className="w-full flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] text-gray-700 hover:bg-gray-50"
            >
              <Icon icon={Settings2} size={14} /> Manage wallets…
            </button>
          </>
        )}
      </Popover>
      {signer.canLock && (
        <Tooltip content={lockTip} className="flex">
          <button
            type="button"
            onClick={() => void signer.toggle()}
            disabled={signer.busy !== null}
            aria-busy={signer.busy !== null}
            aria-label={signer.unlocked ? "Lock the signer" : "Unlock the signer"}
            data-testid="sidebar-lock"
            className={cn(
              "w-[34px] shrink-0 rounded-md border border-gray-200 flex items-center justify-center hover:border-gray-300 disabled:opacity-60",
              signer.unlocked ? "bg-white text-gray-600" : "bg-amber-50 text-amber-800",
            )}
          >
            <Icon icon={signer.unlocked ? LockOpen : Lock} size={15} />
          </button>
        </Tooltip>
      )}
    </div>
  );
}
