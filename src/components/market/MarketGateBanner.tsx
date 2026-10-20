import { Button } from "../ui/Button";
import { BUYING_OFF_LEAD, BUYING_OFF_TEXT } from "./marketText";

/** R15's mainnet gate as a banner whose button turns buying on. */
export function MarketGateBanner({ onTurnOn }: { onTurnOn: () => void }) {
  return (
    <div
      data-testid="market-gate-banner"
      className="flex flex-wrap items-center gap-4 rounded-md border border-blue-200 bg-blue-50 px-3.5 py-2.5 text-[13px] leading-normal text-blue-900"
    >
      <p className="min-w-0 flex-1 [text-wrap:pretty]">
        <strong className="font-semibold">{BUYING_OFF_LEAD}</strong> {BUYING_OFF_TEXT}
      </p>
      <Button
        variant="primary"
        className="shrink-0"
        data-testid="market-gate-turn-on"
        onClick={onTurnOn}
      >
        Turn on buying
      </Button>
    </div>
  );
}
