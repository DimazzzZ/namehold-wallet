import { useEffect, useRef } from "react";
import { Link, Navigate, useParams, useSearchParams } from "react-router-dom";
import { Tabs } from "./ui/Tabs";
import { UnlockButton } from "./UnlockButton";
import { LockDialog } from "./market/LockDialog";
import { sellRefusal } from "./market/marketText";
import { NameHeader } from "./name-page/NameHeader";
import { NameTimeline } from "./name-page/NameTimeline";
import { OverviewTab } from "./name-page/OverviewTab";
import { DnsTab } from "./name-page/DnsTab";
import { TransferTab } from "./name-page/TransferTab";
import { SignTab } from "./name-page/SignTab";
import { useNameActions } from "./name-page/useNameActions";
import { useNameBids } from "../queries/read";
import { useSettingsStore } from "../stores/settings";
import { toAceName } from "../lib/idnEncode";
import { NAMES_ROUTE, namePath, parseNameTab, type NameTab } from "../lib/nameRoute";
import { nameStatusLine, nameTimeline } from "../lib/nameStatus";
import type { WorkspaceTab } from "../types";

/**
 * `/names/:name`. The URL always carries the raw lower-case ACE form; anything
 * else redirects there, so only ACE ever reaches a backend call or query key.
 */
export function NameRoute() {
  const param = useParams().name ?? "";
  const [params] = useSearchParams();
  const ace = toAceName(param);

  if (ace === "") {
    return (
      <div className="flex flex-col gap-5 text-sm">
        <nav className="text-[13px] flex items-center gap-1.5" aria-label="Breadcrumb">
          <Link to={NAMES_ROUTE} className="text-gray-500 hover:text-gray-900 hover:underline">
            Names
          </Link>
          <span className="text-gray-300">/</span>
          <span className="font-mono text-gray-700">.{param}</span>
        </nav>
        <h1 className="font-mono text-3xl font-bold tracking-tight text-gray-900 -mt-2 break-all">
          .{param}
        </h1>
        <div
          className="bg-red-50 border border-red-300 rounded p-3 text-sm text-red-800"
          data-testid="name-invalid"
        >
          This isn&apos;t a valid Handshake name.
        </div>
      </div>
    );
  }
  if (ace !== param) {
    return <Navigate to={namePath(ace, parseNameTab(params.get("tab")))} replace />;
  }
  // A fresh mount per name resets every piece of page state and runs the
  // draft cleanup on leave.
  return <NamePage key={ace} name={ace} />;
}

/** One name: header, lifecycle timeline and four deep-linkable tabs. */
export function NamePage({ name }: { name: string }) {
  const a = useNameActions(name);
  const { caps, info, profile, sections, writeCap } = a;
  const [params, setParams] = useSearchParams();
  const tab = parseNameTab(params.get("tab"));
  const setTab = (k: NameTab) => setParams(k === "overview" ? {} : { tab: k }, { replace: true });
  const experimental = useSettingsStore((s) => s.settings?.shakedex_experimental);
  const { data: bids } = useNameBids(name, profile?.id ?? null);

  const rootRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    rootRef.current?.closest(".overflow-auto")?.scrollTo?.(0, 0);
  }, []);

  const network = profile?.network;
  const status = nameStatusLine({
    caps,
    summary: a.summary,
    info,
    countdown: a.countdown,
    network,
    loading: a.isLoading,
    capsPending: a.capsPending,
    error: a.isError,
  });
  const onChainBids = bids?.bids.filter((b) => !b.pending).length ?? 0;
  const steps = nameTimeline({
    caps,
    info,
    bidCount: onChainBids ? `${onChainBids} bids` : "",
    countdown: a.countdown,
    network,
  });

  const refusal = sellRefusal(profile?.kind, writeCap, network, experimental);
  const canSell =
    sections.ownership.kind === "live" &&
    !caps?.transferPending &&
    refusal === null &&
    !!caps?.canTransfer?.allowed &&
    a.canWrite;

  const hint = "Available after you register this name";
  const tabs: WorkspaceTab<NameTab>[] = [
    { key: "overview", label: "Overview" },
    {
      key: "dns",
      label: "DNS records",
      hint: caps?.ownsName && sections.records.kind !== "live" ? hint : undefined,
    },
    {
      key: "transfer",
      label: "Transfer & sell",
      hint: caps?.ownsName && sections.ownership.kind !== "live" ? hint : undefined,
    },
    {
      key: "sign",
      label: "Sign message",
      hint: caps?.ownsName && sections.ownership.kind !== "live" ? hint : undefined,
    },
  ];

  return (
    <div ref={rootRef} className="flex flex-col gap-5 text-sm">
      <NameHeader
        a={a}
        status={status}
        onTab={setTab}
        onSell={() => a.setSelling(true)}
        canSell={canSell}
      />

      {!a.isLoading && !a.isError && (
        <NameTimeline steps={steps} urgent={caps?.taskState === "expiringSoon"} />
      )}

      {a.isLoading && (
        <div className="text-center py-4">
          <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-blue-600 mx-auto"></div>
          <div className="mt-2 text-sm text-gray-600">Loading name info...</div>
        </div>
      )}

      {a.isError && (
        <div className="bg-red-50 border border-red-300 rounded p-3 text-sm text-red-800">
          <div className="font-medium">Failed to load name info</div>
          <div className="mt-1 text-xs">{a.error?.message || "Unknown error"}</div>
        </div>
      )}

      {a.writeGateVisible && (
        <div
          className="bg-amber-50 border border-amber-200 rounded p-2 text-xs text-amber-800"
          role="status"
          data-testid="name-actions-blocked"
        >
          <div className="flex items-center justify-between gap-2">
            <span>
              <span className="font-semibold">Name actions unavailable.</span>{" "}
              {writeCap?.reason ??
                "Connect a fully-synced, address-indexed node and unlock your signer to manage names."}
            </span>
            {/* Unlock can't help a watch-only wallet. Secondary: the page's
                one primary is the header action. */}
            {!profile?.watchOnly && <UnlockButton size="sm" variant="secondary" />}
          </div>
        </div>
      )}

      <Tabs tabs={tabs} active={tab} onChange={setTab} />

      {tab === "overview" && <OverviewTab a={a} onTab={setTab} />}
      {tab === "dns" && <DnsTab a={a} />}
      {tab === "transfer" && <TransferTab a={a} sellRefusal={refusal} />}
      {tab === "sign" && <SignTab a={a} />}

      {a.selling && <LockDialog open name={name} onClose={() => a.setSelling(false)} />}
    </div>
  );
}
