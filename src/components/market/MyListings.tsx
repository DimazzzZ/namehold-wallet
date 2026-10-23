import { useState } from "react";
import { Alert } from "../ui/Alert";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Disclosure } from "../ui/Disclosure";
import { FeeRateOverride } from "../ui/FeeRateOverride";
import { ActionHint } from "../name-actions/ActionHint";
import { parseFeeRateArg } from "../../lib/feeRate";
import { save } from "../../lib/dialog";
import { useQueryClient } from "@tanstack/react-query";
import { StagedError, mapError, stageOf, unwrapStaged } from "../../lib/errors";
import { formatHns } from "../../lib/utils";
import { displayName } from "../../lib/idn";
import {
  useCancelListing,
  useFinalizeCancel,
  useKeptDraft,
  useLowerPrice,
  useManageListingHere,
  useMyListings,
  usePublishListing,
  useSaveListingFile,
} from "../../queries/shakedex";
import {
  useActiveProfile,
  useBroadcastTxDraft,
  useDeleteTxDraft,
  useExecuteDraft,
  useWithSigner,
  useWriteCapability,
} from "../../queries/wallet";
import { useSettingsStore } from "../../stores/settings";
import { useUiStore } from "../../stores/ui";
import type { ShakedexMyListing } from "../../types";
import { FinalizeAndSign } from "./FinalizeAndSign";
import { FindLocks, OwnFileImport, RestoreLock } from "./RestoreLock";
import {
  CANCEL_MARKET_TOLD,
  CANCEL_MEMPOOL_PURCHASE,
  CANCEL_THEN_FINALIZE,
  CANCELLED_ELSEWHERE,
  EXPORT_THIS_DEVICE_ONLY,
  MANAGED_ELSEWHERE,
  MANAGE_HERE_LABEL,
  MANAGE_HERE_TEXT,
  ON_THIS_DEVICE,
  SOLD_ELSEWHERE,
  cancelElsewhereText,
  managedElsewherePriceText,
  LISTING_BUTTONS,
  LISTING_BUTTON_LABEL,
  LOWER_PRICE_PERMANENCE,
  MANAGED_ELSEWHERE_BUTTONS,
  PUBLISH_PRIVACY,
  TONE_DOT,
  type ListingButton,
  listedUntilText,
  listingPricesText,
  listingStateLabel,
  lowerPriceProblem,
  marketStatusText,
  modeText,
  sellRefusal,
  shakedexRefusal,
  stepTimeText,
  unbuyableText,
  cancelStillBuyable,
  discardedText,
  withKept,
} from "./marketText";

/** A row's dialog. */
type RowDialog = "finalize" | "importFile" | "cancel" | "lower" | "manageHere" | null;

const TEST_ID: Record<ListingButton, string> = {
  finalizeAndSign: "listing-finalize-and-sign",
  lowerPrice: "listing-lower-price",
  cancel: "listing-cancel",
  finalizeCancel: "listing-finalize-cancel",
  export: "listing-export",
  publish: "listing-publish",
  importFile: "listing-import-file",
  manageHere: "listing-manage-here",
};

/**
 * The seller's listings (R25, R26, R28): one row per listing with its
 * state, where it stands on LearnHNS Market, its prices and the actions its
 * state shows. Whether an action can run is the backend's (`actions`, from
 * the refusal each command runs first); the profile's gates come first.
 * "Restore a lock by name" (R32) follows the list, empty or not.
 */
export function MyListings() {
  const listings = useMyListings();
  // A row's gates are the profile's and the node's: no row until both are known.
  const profile = useActiveProfile();
  const writeCap = useWriteCapability();
  if (profile.isError || writeCap.isError) {
    return (
      <Alert tone="error" title="Could not check what this wallet can sign">
        {mapError(profile.error ?? writeCap.error)}
      </Alert>
    );
  }
  if (listings.isLoading || profile.isLoading || writeCap.isLoading) {
    return <p className="text-sm text-gray-500">Loading your listings…</p>;
  }
  const rows = listings.data ?? [];
  return (
    <div>
      {listings.isError ? (
        <Alert tone="error" title="Could not load your listings">
          {mapError(listings.error)}
        </Alert>
      ) : rows.length === 0 ? (
        <p className="text-sm text-gray-500">
          No listings yet. Sell a name from its actions in Owned Names.
        </p>
      ) : (
        <ul className="divide-y divide-gray-100 border-y border-gray-100 bg-white">
          {rows.map((row) => (
            <ListingRow key={row.listing.id} row={row} />
          ))}
        </ul>
      )}
      <FindLocks />
      <RestoreLock />
    </div>
  );
}

/** One listing of My listings, with its actions and their dialogs. */
function ListingRow({ row }: { row: ShakedexMyListing }) {
  const { listing, actions, prices, pendingSend } = row;
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const { data: writeCap } = useWriteCapability();
  const experimental = useSettingsStore((s) => s.settings?.shakedex_experimental);
  const withSigner = useWithSigner();
  const exec = useExecuteDraft();
  const broadcast = useBroadcastTxDraft();
  const deleteDraft = useDeleteTxDraft();
  const finalizeCancel = useFinalizeCancel();
  const publish = usePublishListing();
  const saveFile = useSaveListingFile();
  const keptDraft = useKeptDraft();
  const qc = useQueryClient();
  const [dialog, setDialog] = useState<RowDialog>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mainnet = profile?.network === "mainnet";
  // R16, R6: what signs and sends needs a recovery-phrase wallet on a node
  // that can send. Export only reads the stored file.
  const signingGate = shakedexRefusal(profile?.kind, writeCap);
  // Publishing starts a market relation, as a new listing does (R23, R29).
  const publishGate = sellRefusal(profile?.kind, writeCap, profile?.network, experimental);
  const gateOf = (b: ListingButton) =>
    b === "export" ? null : b === "publish" ? publishGate : signingGate;

  const elsewhere = row.managedElsewhere;
  const buttons = [
    ...LISTING_BUTTONS[listing.state],
    // R44, R45: a Listed row is taken over, by the button or the other
    // device's file, only while another device manages it.
    ...(listing.state === "listed" && elsewhere ? MANAGED_ELSEWHERE_BUTTONS : []),
  ].filter(
    // Carry 1: off → on only, on mainnet only.
    (b) => b !== "publish" || (!listing.publish && mainnet),
  );

  /** Run a row action; its failure stays on the row. */
  const act = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(mapError(e));
    } finally {
      setBusy(false);
    }
  };

  const exportFile = () =>
    act(async () => {
      const path = await save({
        title: "Save the listing file",
        defaultPath: `${listing.name}-listing.json`,
        filters: [{ name: "Listing file", extensions: ["json"] }],
      });
      if (!path) return;
      // Saving the stored file signs nothing: no signer, any wallet.
      await saveFile.mutateAsync({ listingId: listing.id, path });
      showToast("Listing file saved");
    });

  const publishNow = () =>
    act(async () => {
      if (!profile) return;
      await withSigner.run(profile.id, () => publish.mutateAsync(listing.id));
      showToast("Published. It goes on LearnHNS Market at the next sync.");
    });

  /**
   * A failed sign or send, with its stage; after a failed broadcast, where
   * the draft is kept when a fresh read of the rows still offers it.
   */
  const sendError = async (e: unknown, draftId: string) => {
    const stage = stageOf(e);
    const why = mapError(unwrapStaged(e), stage);
    return stage === "broadcast" ? withKept(why, await keptDraft(draftId)) : why;
  };

  const sendPending = async () => {
    if (!profile || !pendingSend) return;
    const id = pendingSend.draftId;
    setBusy(true);
    setError(null);
    try {
      await withSigner.run(profile.id, async () => {
        if (!pendingSend.signed) return exec.run(id, profile.id, true);
        try {
          return await broadcast.mutateAsync(id);
        } catch (e) {
          throw new StagedError("broadcast", e);
        }
      });
      qc.invalidateQueries({ queryKey: ["read"] });
      showToast("Sent");
    } catch (e) {
      // The row's own draft: it stays for Send or Discard.
      setError(await sendError(e, id));
    } finally {
      setBusy(false);
    }
  };

  // The backend puts the listing back where it was before the draft
  // (`delete_tx_draft`): a Locking listing goes with its lock.
  const discardPending = () =>
    act(async () => {
      if (!pendingSend) return;
      await deleteDraft.mutateAsync(pendingSend.draftId);
      qc.invalidateQueries({ queryKey: ["read"] });
      showToast(discardedText(listing.state));
    });

  const finalizeHome = async () => {
    if (!profile) return;
    setBusy(true);
    setError(null);
    let draftId: string;
    try {
      const draft = await withSigner.run(profile.id, () =>
        finalizeCancel.mutateAsync({ listingId: listing.id, feeRate: null }),
      );
      draftId = draft.id;
    } catch (e) {
      setError(mapError(e));
      setBusy(false);
      return;
    }
    try {
      await withSigner.run(profile.id, () => exec.run(draftId, profile.id, true));
      showToast("Sent. The name comes home once the finalize is mined.");
    } catch (e) {
      setError(await sendError(e, draftId));
      // Unsent, the listing awaits its finalize again (`delete_tx_draft`).
      if (stageOf(e) !== "broadcast") {
        try {
          await deleteDraft.mutateAsync(draftId);
        } catch {
          // Best-effort: the row offers it to send or discard.
        }
      }
    } finally {
      setBusy(false);
    }
  };

  const onClick: Record<ListingButton, () => void> = {
    finalizeAndSign: () => setDialog("finalize"),
    importFile: () => setDialog("importFile"),
    cancel: () => setDialog("cancel"),
    lowerPrice: () => setDialog("lower"),
    manageHere: () => setDialog("manageHere"),
    finalizeCancel: () => void finalizeHome(),
    export: () => void exportFile(),
    publish: () => void publishNow(),
  };

  const mode = modeText(listing);
  // R42: a listing managed elsewhere says so in place of its market status.
  const status = elsewhere
    ? { tone: "warn" as const, text: MANAGED_ELSEWHERE }
    : marketStatusText(listing, profile?.network);
  const ownPrices = listingPricesText(listing, prices);
  // R42: this device's own prices, never confused with the other device's.
  const pricesText = ownPrices && elsewhere ? `${ownPrices} — ${ON_THIS_DEVICE}` : ownPrices;
  // R43: chain facts from another device, in words only.
  const fromElsewhere =
    listing.state === "sold" && row.soldElsewhere
      ? SOLD_ELSEWHERE
      : row.cancelledElsewhere
        ? CANCELLED_ELSEWHERE
        : null;
  const listedUntil =
    listing.state === "listed" && listing.expiresAt != null
      ? listedUntilText(listing.expiresAt)
      : null;
  // R35: buyers stop seeing a lock whose name gets too close to expiring.
  const unbuyable = unbuyableText(
    listing.blocksUntilUnbuyable,
    row.unbuyableWarnBlocks,
    row.wallClockTiming,
  );

  // Each disabled button's reason, once: the gate first, then the backend's.
  const reasons = new Set<string>();
  const button = (b: ListingButton) => {
    const action = actions[b];
    const gate = gateOf(b);
    const reason = gate ?? (action.allowed ? null : action.reason);
    if (reason) reasons.add(reason);
    return (
      <ActionHint key={b} reason={reason}>
        <Button
          size="sm"
          data-testid={TEST_ID[b]}
          disabled={busy || gate !== null || !action.allowed}
          onClick={onClick[b]}
        >
          {LISTING_BUTTON_LABEL[b]}
        </Button>
      </ActionHint>
    );
  };
  const rendered = buttons.map(button);
  if (pendingSend && signingGate) reasons.add(signingGate);

  return (
    <li className="px-3 py-3" data-testid="my-listing-row">
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
        <span className="font-mono text-sm font-medium text-gray-900">
          .{displayName(listing.name)}
        </span>
        {mode && <span className="text-xs text-gray-500">{mode}</span>}
        <Badge>{listingStateLabel(listing.state, listing.blocksUntilFinalize)}</Badge>
      </div>
      <div className="mt-1 flex items-start gap-2 text-sm text-gray-700">
        <span
          aria-hidden
          className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${TONE_DOT[status.tone]}`}
        />
        <span data-testid="listing-market-status">{status.text}</span>
      </div>
      {elsewhere && row.foreignStepPriceDoos != null && (
        <div className="mt-1 text-sm text-gray-700" data-testid="listing-elsewhere-price">
          {managedElsewherePriceText(row.foreignStepPriceDoos)}
        </div>
      )}
      {fromElsewhere && (
        <div className="mt-1 text-xs text-gray-600" data-testid="listing-from-elsewhere">
          {fromElsewhere}
        </div>
      )}
      {pricesText && (
        <div className="mt-1 text-sm tabular-nums text-gray-900" data-testid="listing-prices">
          {pricesText}
        </div>
      )}
      {listing.state === "listed" && listing.steps.length > 1 && (
        <Disclosure
          summary={`All ${listing.steps.length} price steps`}
          className="mt-1 text-xs"
          testId="listing-steps-toggle"
        >
          <ul className="space-y-0.5 text-xs tabular-nums text-gray-600">
            {listing.steps.map((s, i) => (
              <li key={i}>
                {formatHns(s.price)} HNS — from{" "}
                {stepTimeText(s.lockTime) ?? "a time too far away to show"}
              </li>
            ))}
          </ul>
        </Disclosure>
      )}
      {listedUntil && <div className="mt-1 text-xs text-gray-400">{listedUntil}</div>}
      {unbuyable && (
        <p className="mt-1 text-xs text-amber-800" data-testid="listing-unbuyable">
          {unbuyable}
        </p>
      )}
      {(rendered.length > 0 || pendingSend) && (
        <div className="mt-2 flex flex-wrap gap-2">
          {pendingSend && (
            <>
              <ActionHint reason={signingGate}>
                <Button
                  size="sm"
                  variant="primary"
                  data-testid="listing-send"
                  disabled={busy || signingGate !== null}
                  onClick={() => void sendPending()}
                >
                  {pendingSend.signed ? "Send" : "Sign and send"}
                </Button>
              </ActionHint>
              {/* A send the node may hold (`broadcast_pending`) cannot be discarded. */}
              {pendingSend.discardable && (
                <Button
                  size="sm"
                  variant="ghost"
                  data-testid="listing-discard"
                  disabled={busy}
                  onClick={() => void discardPending()}
                >
                  Discard
                </Button>
              )}
            </>
          )}
          {rendered}
        </div>
      )}
      {buttons.includes("publish") && (
        <p className="mt-1 text-xs text-gray-500">{PUBLISH_PRIVACY}</p>
      )}
      {elsewhere && buttons.includes("export") && (
        <p className="mt-1 text-xs text-gray-500">{EXPORT_THIS_DEVICE_ONLY}</p>
      )}
      {[...reasons].map((r) => (
        <p key={r} className="mt-1 text-xs text-gray-600">
          {r}
        </p>
      ))}
      {error && (
        <Alert tone="error" className="mt-2">
          <span data-testid="listing-error">{error}</span>
        </Alert>
      )}
      {dialog === "finalize" && (
        <FinalizeAndSign open listing={listing} onClose={() => setDialog(null)} />
      )}
      {dialog === "importFile" && (
        <OwnFileImport listing={listing} onClose={() => setDialog(null)} />
      )}
      {dialog === "cancel" && <CancelDialog row={row} onClose={() => setDialog(null)} />}
      {dialog === "lower" && <LowerPriceDialog row={row} onClose={() => setDialog(null)} />}
      {dialog === "manageHere" && <ManageHereDialog row={row} onClose={() => setDialog(null)} />}
    </li>
  );
}

/**
 * R44: take the listing over from the other device, its price included.
 * The backend re-reads the lock coin and the step; nothing is signed.
 */
function ManageHereDialog({ row, onClose }: { row: ShakedexMyListing; onClose: () => void }) {
  const { listing } = row;
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const withSigner = useWithSigner();
  const manage = useManageListingHere();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const confirm = async () => {
    if (!profile) return;
    setBusy(true);
    setError(null);
    try {
      await withSigner.run(profile.id, () => manage.mutateAsync(listing.id));
      showToast("This device manages the listing again. It goes up at the next sync.");
      onClose();
    } catch (e) {
      setError(mapError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onClose={() => !busy && onClose()}
      title={`${MANAGE_HERE_LABEL}: .${displayName(listing.name)}`}
    >
      <div className="space-y-3 text-sm text-gray-700">
        <p data-testid="manage-here-text">{MANAGE_HERE_TEXT(row.foreignStepPriceDoos ?? 0)}</p>
        {error && (
          <Alert tone="error">
            <span data-testid="manage-here-error">{error}</span>
          </Alert>
        )}
        <div className="flex justify-end gap-2 pt-2">
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            Leave it
          </Button>
          <Button
            variant="primary"
            data-testid="manage-here-confirm"
            disabled={busy}
            onClick={() => void confirm()}
          >
            {busy ? "Processing…" : MANAGE_HERE_LABEL}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

/**
 * R28: what a cancel cannot stop, then the cancel signed by the backend
 * (its secure window shows the current price and the fee) and sent.
 */
function CancelDialog({ row, onClose }: { row: ShakedexMyListing; onClose: () => void }) {
  const { listing } = row;
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const withSigner = useWithSigner();
  const cancel = useCancelListing();
  const broadcast = useBroadcastTxDraft();
  const keptDraft = useKeptDraft();
  const [signed, setSigned] = useState(false);
  const [feeRateRaw, setFeeRateRaw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const sign = async () => {
    if (!profile) return;
    setBusy(true);
    setError(null);
    let draftId: string;
    try {
      const d = await withSigner.run(profile.id, () =>
        cancel.mutateAsync({ listingId: listing.id, feeRate: parseFeeRateArg(feeRateRaw) }),
      );
      draftId = d.id;
    } catch (e) {
      setError(mapError(e));
      setBusy(false);
      return;
    }
    try {
      await broadcast.mutateAsync(draftId);
      showToast("Cancel sent. Finalize the name home once it is mined and the lockup is over.");
      onClose();
    } catch (e) {
      // The signed cancel is not this dialog's to delete or to sign again:
      // the row offers it, when the backend kept it.
      setSigned(true);
      setError(withKept(mapError(e, "broadcast"), await keptDraft(draftId)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onClose={() => !busy && onClose()}
      title={`Cancel the listing of .${displayName(listing.name)}`}
    >
      <div className="space-y-3 text-sm text-gray-700">
        <p>{cancelStillBuyable(listing, row.prices)}</p>
        {row.managedElsewhere && row.foreignStepPriceDoos != null && (
          <p data-testid="cancel-elsewhere">{cancelElsewhereText(row.foreignStepPriceDoos)}</p>
        )}
        <p>{CANCEL_MEMPOOL_PURCHASE}</p>
        <p>{CANCEL_THEN_FINALIZE}</p>
        {listing.publish && <p>{CANCEL_MARKET_TOLD}</p>}
        <p className="text-xs text-gray-500">
          The secure window shows the current price and the fee before anything is signed.
        </p>
        <FeeRateOverride value={feeRateRaw} onChange={setFeeRateRaw} label="Fee rate override" />
        {error && (
          <Alert tone="error">
            <span data-testid="cancel-error">{error}</span>
          </Alert>
        )}
        <div className="flex justify-end gap-2 pt-2">
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            Keep the listing
          </Button>
          {!signed && (
            <Button variant="danger" data-testid="cancel-sign" disabled={busy} onClick={sign}>
              {busy ? "Processing…" : "Sign and send the cancel"}
            </Button>
          )}
        </div>
      </div>
    </Dialog>
  );
}

/** R26: one more, lower price; the backend signs it after its secure window. */
function LowerPriceDialog({ row, onClose }: { row: ShakedexMyListing; onClose: () => void }) {
  const { listing, prices } = row;
  const showToast = useUiStore((s) => s.showToast);
  const { data: profile } = useActiveProfile();
  const withSigner = useWithSigner();
  const lower = useLowerPrice();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const current = prices.kind === "at" ? prices.currentPrice : null;
  const problem = text === "" ? null : lowerPriceProblem(text, current);

  const submit = async () => {
    if (!profile) return;
    setBusy(true);
    setError(null);
    try {
      await withSigner.run(profile.id, () =>
        lower.mutateAsync({ listingId: listing.id, price: text }),
      );
      showToast("Price lowered");
      onClose();
    } catch (e) {
      setError(mapError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onClose={() => !busy && onClose()}
      title={`Lower the price of .${displayName(listing.name)}`}
    >
      <div className="space-y-3 text-sm text-gray-700">
        {current != null && <p>Current price: {formatHns(current)} HNS</p>}
        <label className="block">
          <span className="text-gray-700">New price (HNS)</span>
          <input
            data-testid="lower-price-input"
            className="mt-1 block w-full rounded-md border border-gray-300 px-3 py-1.5 text-sm"
            inputMode="decimal"
            value={text}
            disabled={busy}
            onChange={(e) => setText(e.target.value)}
          />
        </label>
        {problem && <p className="text-xs text-red-600">{problem}</p>}
        <p className="text-xs text-gray-500">{LOWER_PRICE_PERMANENCE}</p>
        {error && (
          <Alert tone="error">
            <span data-testid="lower-price-error">{error}</span>
          </Alert>
        )}
        <div className="flex justify-end gap-2 pt-2">
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            Close
          </Button>
          <Button
            variant="primary"
            data-testid="lower-price-submit"
            disabled={busy || text === "" || problem !== null}
            onClick={submit}
          >
            {busy ? "Processing…" : "Sign the lower price"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
