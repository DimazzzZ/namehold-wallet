import { Link } from "react-router-dom";
import { ArrowUpRight, Ellipsis } from "lucide-react";
import { Button } from "../ui/Button";
import { Icon } from "../ui/Icon";
import { Popover } from "../ui/Popover";
import { Tooltip } from "../ui/Tooltip";
import { ActionHint } from "../name-actions/ActionHint";
import { WatchlistToggle } from "../WatchlistToggle";
import { displayName } from "../../lib/idn";
import { NAMES_ROUTE, type NameTab } from "../../lib/nameRoute";
import { STATUS_DOT, type StatusLine } from "../../lib/nameStatus";
import { explorerCoversNetwork, explorerNameUrl, openExternal } from "../../lib/openExternal";
import type { NameActions } from "./useNameActions";
import type { NameActionCapability } from "../../types";

interface Primary {
  label: string;
  busyLabel: string;
  busyKey: string;
  cap: NameActionCapability | undefined;
  extraReason?: string | null;
  onClick: () => void;
}

/** The one action that moves the name forward, per task state, or null. */
function headerPrimary(a: NameActions, onTab: (t: NameTab) => void): Primary | null {
  const { caps, build, name, run } = a;
  if (!caps) return null;
  switch (caps.taskState) {
    case "availableToOpen":
      return {
        label: "Open auction",
        busyLabel: "Opening…",
        busyKey: "OPEN",
        cap: caps.canOpen,
        onClick: () => run("OPEN", () => build.open.mutateAsync({ name })),
      };
    case "readyToReveal":
      return {
        label: "Reveal",
        busyLabel: "…",
        busyKey: "REVEAL",
        cap: caps.canReveal,
        extraReason: caps.hasBidCommitment ? null : "Recover your bid commitment first (below).",
        onClick: () => {
          onTab("overview");
          a.setRevealConfirming(true);
        },
      };
    case "wonNeedsRegister":
      return {
        label: "Register",
        busyLabel: "Registering…",
        busyKey: "REGISTER",
        cap: caps.canRegister,
        onClick: () => a.submitRecords("REGISTER"),
      };
    case "lostNeedsRedeem":
      return {
        label: "Redeem",
        busyLabel: "…",
        busyKey: "REDEEM",
        cap: caps.canRedeem,
        onClick: () => run("REDEEM", () => build.redeem.mutateAsync({ name })),
      };
    case "transferPendingFinalize":
      return {
        label: "Finalize transfer",
        busyLabel: "…",
        busyKey: "FINALIZE",
        cap: caps.canFinalize,
        onClick: () => run("FINALIZE", () => build.finalize.mutateAsync({ name })),
      };
    case "ownedNoUrgentAction":
    case "expiringSoon":
      return {
        label: "Renew",
        busyLabel: "…",
        busyKey: "RENEW",
        cap: caps.canRenew,
        onClick: () => run("RENEW", () => build.renew.mutateAsync({ name })),
      };
    default:
      // In BIDDING the bid form's Bid is the page's one blue button.
      return null;
  }
}

function MenuItem({
  onClick,
  danger,
  children,
}: {
  onClick: () => void;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      className={`block w-full px-3 py-1.5 text-left text-[13px] hover:bg-gray-100 ${
        danger ? "text-red-700" : "text-gray-700"
      }`}
    >
      {children}
    </button>
  );
}

/** Breadcrumb, H1, status line, header buttons and the ⋯ menu. */
export function NameHeader({
  a,
  status,
  onTab,
  onSell,
  canSell,
}: {
  a: NameActions;
  status: StatusLine;
  onTab: (tab: NameTab) => void;
  onSell: () => void;
  canSell: boolean;
}) {
  const { name, caps, profile, writeCap, canWrite, busy } = a;
  const decoded = displayName(name);

  const primary =
    !caps?.pendingBroadcastAction &&
    !a.isLoading &&
    !a.isError &&
    !a.revealConfirming &&
    !a.optimisticRevealTxid
      ? headerPrimary(a, onTab)
      : null;
  const primaryReason = primary
    ? !canWrite
      ? (writeCap?.reason ?? "Unlock your wallet to sign.")
      : (primary.extraReason ?? (primary.cap && !primary.cap.allowed ? primary.cap.reason : null))
    : null;
  const primaryEnabled =
    !!primary && canWrite && !busy && !primary.extraReason && !!primary.cap?.allowed;

  const revokeAllowed = !a.actionDisabled("REVOKE", caps?.canRevoke);

  return (
    <>
      <nav className="text-[13px] flex items-center gap-1.5" aria-label="Breadcrumb">
        <Link to={NAMES_ROUTE} className="text-gray-500 hover:text-gray-900 hover:underline">
          Names
        </Link>
        <span className="text-gray-300">/</span>
        <span className="font-mono text-gray-700">.{decoded}</span>
      </nav>

      <div className="flex flex-wrap items-center justify-between gap-4 -mt-2">
        <div className="min-w-0">
          <h1 className="font-mono text-3xl font-bold tracking-tight text-gray-900 break-all">
            .{decoded}
            {decoded !== name && (
              <span className="ml-2.5 font-sans text-sm font-normal text-gray-400">.{name}</span>
            )}
          </h1>
          <div className="mt-1.5 flex items-center gap-2 text-sm" data-testid="name-phase">
            <span className={`h-2 w-2 shrink-0 rounded-full ${STATUS_DOT[status.tone]}`} />
            <span>
              <b className="font-semibold text-gray-900">
                {status.status === "Checking…" ? (
                  <span data-testid="name-phase-loading">Checking…</span>
                ) : (
                  status.status
                )}
              </b>
              {status.sub && <span className="text-gray-500"> · {status.sub}</span>}
              {status.note && <span className="text-gray-400"> {status.note}</span>}
            </span>
          </div>
        </div>

        <div className="flex items-center gap-2 relative">
          {explorerCoversNetwork(profile?.network) && (
            <Button
              variant="secondary"
              onClick={() => openExternal(explorerNameUrl(name))}
              data-testid="name-explorer-link"
            >
              View on explorer
              <Icon icon={ArrowUpRight} size={14} />
            </Button>
          )}
          <WatchlistToggle name={name} />
          <Popover
            align="right"
            panelClassName="left-auto right-0 w-60"
            aria-label="More actions"
            data-testid="name-more-menu"
            trigger={({ toggle }) => (
              <Tooltip content="More actions">
                <Button
                  variant="secondary"
                  aria-label="More actions"
                  onClick={toggle}
                  data-testid="name-more"
                >
                  <Icon icon={Ellipsis} size={16} />
                </Button>
              </Tooltip>
            )}
          >
            {({ close }) => (
              <>
                <MenuItem
                  onClick={() => {
                    close();
                    onTab("transfer");
                  }}
                >
                  Transfer to another wallet…
                </MenuItem>
                <MenuItem
                  onClick={() => {
                    close();
                    if (canSell) onSell();
                    else onTab("transfer");
                  }}
                >
                  Sell on the market…
                </MenuItem>
                <MenuItem
                  onClick={() => {
                    close();
                    onTab("dns");
                  }}
                >
                  DNS records
                </MenuItem>
                <MenuItem
                  onClick={() => {
                    close();
                    onTab("sign");
                  }}
                >
                  Sign a message…
                </MenuItem>
                <div className="my-1 border-t border-gray-100" role="separator" />
                <MenuItem
                  danger
                  onClick={() => {
                    close();
                    onTab("transfer");
                    if (revokeAllowed) a.setRevokeOpen(true);
                  }}
                >
                  Revoke…
                </MenuItem>
              </>
            )}
          </Popover>
          {primary && (
            <ActionHint reason={primaryReason}>
              <Button
                variant="primary"
                disabled={!primaryEnabled}
                onClick={primary.onClick}
                data-testid="name-primary"
              >
                {busy === primary.busyKey ? primary.busyLabel : primary.label}
              </Button>
            </ActionHint>
          )}
        </div>
      </div>
    </>
  );
}
