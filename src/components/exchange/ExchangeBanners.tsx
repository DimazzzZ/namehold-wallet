import { GatedButton } from "./GatedButton";
import { ENABLE_LEAD, ENABLE_TEXT } from "../../lib/exchangeText";

/** R17: the backend's refusal, verbatim. */
export function ExchangeGateBanner({ refusal }: { refusal: string }) {
  return (
    <div
      role="status"
      data-testid="exchange-gate-banner"
      className="rounded-md border border-amber-200 bg-amber-50 px-3.5 py-2.5 text-[13px] leading-normal text-amber-900"
    >
      <p className="[text-wrap:pretty]">{refusal}</p>
    </div>
  );
}

/** The exchange is not enabled on this profile yet: the one call to action. */
export function EnableExchangeBanner({
  refusal,
  loading,
  onEnable,
}: {
  /** Why enabling is not possible here (the gate's refusal), or null. */
  refusal: string | null;
  loading: boolean;
  onEnable: () => void;
}) {
  return (
    <div
      data-testid="exchange-enable-banner"
      className="flex flex-wrap items-center gap-4 rounded-md border border-blue-200 bg-blue-50 px-3.5 py-2.5 text-[13px] leading-normal text-blue-900"
    >
      <p className="min-w-0 flex-1 [text-wrap:pretty]">
        <strong className="font-semibold">{ENABLE_LEAD}</strong> {ENABLE_TEXT}
      </p>
      <GatedButton
        variant="primary"
        wrapClassName="inline-flex shrink-0"
        data-testid="exchange-enable"
        refusal={refusal}
        loading={loading}
        onClick={onEnable}
      >
        Enable exchange
      </GatedButton>
    </div>
  );
}
