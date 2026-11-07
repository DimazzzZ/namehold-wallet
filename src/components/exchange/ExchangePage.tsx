import { useState } from "react";
import { useSearchParams } from "react-router-dom";
import { PageHeader } from "../ui/PageHeader";
import { Tabs } from "../ui/Tabs";
import { Alert } from "../ui/Alert";
import { Icon } from "../ui/Icon";
import { Settings as SettingsIcon } from "lucide-react";
import { GatedButton } from "./GatedButton";
import { EnableExchangeBanner, ExchangeGateBanner } from "./ExchangeBanners";
import { OrderBook } from "./OrderBook";
import { MyOffers } from "./MyOffers";
import { MyTrades } from "./MyTrades";
import { BtcAccountCard, BtcWithdrawDialog } from "./BtcAccountCard";
import { TakeOfferDialog } from "./TakeOfferDialog";
import { CreateOfferDialog } from "./CreateOfferDialog";
import { ExchangeSettingsDialog } from "./ExchangeSettingsDialog";
import {
  useBtcBalance,
  useBtcWithdraw,
  useCancelTrade,
  useCommitTrade,
  useCreateOffer,
  useEnableExchange,
  useExchangeOverview,
  useMarketOffers,
  useMyOffers,
  useRefreshTrades,
  useTakeOffer,
  useTrades,
  useUpdateExchangeSettings,
  useWithdrawOffer,
} from "../../queries/exchange";
import { useUiStore } from "../../stores/ui";
import { mapError } from "../../lib/errors";
import { needsMyCommit } from "../../lib/exchangeStatus";
import {
  BTC_SENT_TOAST,
  CANCELLED_TOAST,
  COMMITTED_TOAST,
  ENABLED_TOAST,
  EXCHANGE_SUBTITLE,
  EXCHANGE_TITLE,
  OFFER_CREATED_TOAST,
  NO_RELAY,
  OFFER_WITHDRAWN_TOAST,
  SETTINGS_SAVED_TOAST,
  TAKEN_TOAST,
  exchangeActionRefusal,
  exchangeRefusal,
  hasRelay,
  offerActionRefusal,
  takeRefusal,
} from "../../lib/exchangeText";
import type { OfferRow, TradeRow } from "../../types/exchange";
import type { WorkspaceTab } from "../../types";

type ExchangeTab = "book" | "offers" | "trades";

function tabOf(param: string | null): ExchangeTab {
  return param === "offers" || param === "trades" ? param : "book";
}

/** The Exchange: the HNS ⇄ BTC order book, the user's offers and trades, and the BTC account. */
export default function ExchangePage() {
  const [params, setParams] = useSearchParams();
  const tab = tabOf(params.get("tab"));
  const showToast = useUiStore((s) => s.showToast);

  const overviewQ = useExchangeOverview();
  const overview = overviewQ.data;
  const enabled = overview?.enabled === true;
  const relay = hasRelay(overview);
  // Browsing the order book needs no keys, only a relay; the user's own data needs the account.
  const offersQ = useMarketOffers(overview != null && relay);
  const myOffersQ = useMyOffers(enabled);
  const tradesQ = useTrades(enabled);
  const balanceQ = useBtcBalance(enabled);

  const enable = useEnableExchange();
  const createOffer = useCreateOffer();
  const withdrawOffer = useWithdrawOffer();
  const takeOffer = useTakeOffer();
  const commit = useCommitTrade();
  const cancel = useCancelTrade();
  const refresh = useRefreshTrades();
  const btcWithdraw = useBtcWithdraw();
  const updateSettings = useUpdateExchangeSettings();

  const [taking, setTaking] = useState<OfferRow | null>(null);
  const [creating, setCreating] = useState(false);
  const [withdrawingBtc, setWithdrawingBtc] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // One predicate for every button on the page (R17).
  const refusal = exchangeRefusal(overview);
  const actionRefusal = exchangeActionRefusal(overview);
  const offerRefusal = offerActionRefusal(overview);

  const trades = tradesQ.data ?? [];
  const waiting = trades.filter(needsMyCommit).length;
  const tabs: WorkspaceTab<ExchangeTab>[] = [
    { key: "book", label: "Order book" },
    { key: "offers", label: "My offers", badge: myOffersQ.data?.length || undefined },
    {
      key: "trades",
      label: "My trades",
      badge: waiting || undefined,
      description: waiting ? `${waiting} waiting for your confirmation` : undefined,
    },
  ];

  const fail = (e: unknown) => showToast(mapError(e), "error");

  const onEnable = () =>
    enable.mutate(undefined, {
      onSuccess: () => showToast(ENABLED_TOAST, "success"),
      onError: fail,
    });

  const onCommit = (t: TradeRow) =>
    commit.mutate(t.tradeId, {
      onSuccess: () => showToast(COMMITTED_TOAST, "success"),
      onError: fail,
    });

  const onCancel = (t: TradeRow) =>
    cancel.mutate(t.tradeId, {
      onSuccess: () => showToast(CANCELLED_TOAST, "success"),
      onError: fail,
    });

  const onWithdrawOffer = (o: OfferRow) =>
    withdrawOffer.mutate(o.id, {
      onSuccess: () => showToast(OFFER_WITHDRAWN_TOAST, "success"),
      onError: fail,
    });

  return (
    <div>
      <PageHeader
        title={EXCHANGE_TITLE}
        subtitle={EXCHANGE_SUBTITLE}
        aside={
          <>
            <GatedButton
              variant="secondary"
              data-testid="exchange-refresh"
              refusal={actionRefusal}
              loading={refresh.isPending}
              onClick={() => refresh.mutate(undefined, { onError: fail })}
            >
              Refresh
            </GatedButton>
            <GatedButton
              variant="secondary"
              data-testid="exchange-create-offer"
              refusal={offerRefusal}
              onClick={() => setCreating(true)}
            >
              Make an offer…
            </GatedButton>
            <GatedButton
              variant="secondary"
              data-testid="exchange-settings"
              refusal={refusal}
              onClick={() => setSettingsOpen(true)}
            >
              <Icon icon={SettingsIcon} size={14} />
              Settings
            </GatedButton>
          </>
        }
      />
      <div className="space-y-4">
        {overviewQ.isError && (
          <Alert tone="error" title="Couldn't check the exchange">
            <span data-testid="exchange-overview-error">{mapError(overviewQ.error)}</span>
          </Alert>
        )}
        {overview && refusal !== null && <ExchangeGateBanner refusal={refusal} />}
        {overview && !overview.enabled && (
          <EnableExchangeBanner refusal={refusal} loading={enable.isPending} onEnable={onEnable} />
        )}
        <div className="flex flex-wrap items-start gap-5">
          <div className="min-w-0 flex-[999_1_540px]">
            <Tabs
              className="mb-4"
              tabs={tabs}
              active={tab}
              onChange={(k) => setParams(k === "book" ? {} : { tab: k })}
            />
            {tab === "book" && overview && !relay && (
              <p
                className="rounded-lg border border-dashed border-gray-300 bg-white px-4 py-6 text-center text-sm text-gray-600"
                data-testid="order-book-no-relay"
              >
                {NO_RELAY}
              </p>
            )}
            {tab === "book" && (!overview || relay) && (
              <OrderBook
                overview={overview}
                market={offersQ.data}
                loading={offersQ.isPending}
                error={offersQ.error}
                onTake={setTaking}
              />
            )}
            {tab === "offers" && (
              <MyOffers
                offers={myOffersQ.data ?? []}
                loading={enabled && myOffersQ.isPending}
                error={myOffersQ.error}
                refusal={offerRefusal}
                withdrawingId={withdrawOffer.isPending ? (withdrawOffer.variables ?? null) : null}
                onWithdraw={onWithdrawOffer}
              />
            )}
            {tab === "trades" && (
              <MyTrades
                trades={trades}
                loading={enabled && tradesQ.isPending}
                error={tradesQ.error}
                refusal={actionRefusal}
                committingId={commit.isPending ? (commit.variables ?? null) : null}
                cancellingId={cancel.isPending ? (cancel.variables ?? null) : null}
                onCommit={onCommit}
                onCancel={onCancel}
              />
            )}
          </div>
          <div className="w-full flex-[1_1_300px] lg:max-w-sm">
            <BtcAccountCard
              enabled={enabled}
              balance={balanceQ.data}
              loading={enabled && balanceQ.isPending}
              error={balanceQ.error}
              refusal={actionRefusal}
              onWithdraw={() => setWithdrawingBtc(true)}
            />
          </div>
        </div>
      </div>

      {/* Mounted while open only: each opening starts empty. */}
      {taking && (
        <TakeOfferDialog
          offer={taking}
          refusal={takeRefusal(overview, taking)}
          busy={takeOffer.isPending}
          onClose={() => setTaking(null)}
          onTake={async (hnsAmount) => {
            await takeOffer.mutateAsync({ offerId: taking.id, hnsAmount });
            setTaking(null);
            setParams({ tab: "trades" });
            showToast(TAKEN_TOAST, "success");
          }}
        />
      )}
      {creating && (
        <CreateOfferDialog
          refusal={offerRefusal}
          busy={createOffer.isPending}
          onClose={() => setCreating(false)}
          onCreate={async (args) => {
            await createOffer.mutateAsync(args);
            setCreating(false);
            setParams({ tab: "offers" });
            showToast(OFFER_CREATED_TOAST, "success");
          }}
        />
      )}
      {settingsOpen && overview && (
        <ExchangeSettingsDialog
          overview={overview}
          refusal={refusal}
          busy={updateSettings.isPending}
          onClose={() => setSettingsOpen(false)}
          onSave={async (args) => {
            await updateSettings.mutateAsync(args);
            setSettingsOpen(false);
            showToast(SETTINGS_SAVED_TOAST, "success");
          }}
        />
      )}
      {withdrawingBtc && (
        <BtcWithdrawDialog
          balance={balanceQ.data}
          refusal={actionRefusal}
          busy={btcWithdraw.isPending}
          onClose={() => setWithdrawingBtc(false)}
          onWithdraw={async (args) => {
            const { txid } = await btcWithdraw.mutateAsync(args);
            setWithdrawingBtc(false);
            showToast(`${BTC_SENT_TOAST} · ${txid.slice(0, 12)}…`, "success");
          }}
        />
      )}
    </div>
  );
}
