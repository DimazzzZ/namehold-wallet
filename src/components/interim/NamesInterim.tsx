// Interim (shell-v2-2026-10-28). Replaced by names-page-2026-10-28; delete when that lands.
import { useSearchParams } from "react-router-dom";
import { AuctionsView } from "../AuctionsView";
import { Watchlist } from "../Watchlist";
import { Tabs } from "../ui/Tabs";

/** /names until the real Names page lands: Auctions | Watchlist, bound to ?filter. */
export function NamesInterim() {
  const [params, setParams] = useSearchParams();
  const tab = params.get("filter") === "watching" ? "watching" : "auction";
  return (
    <div className="flex flex-col gap-5">
      <Tabs
        tabs={[
          { key: "auction", label: "Auctions" },
          { key: "watching", label: "Watchlist" },
        ]}
        active={tab}
        onChange={(filter) => setParams({ filter }, { replace: true })}
      />
      {tab === "watching" ? <Watchlist /> : <AuctionsView />}
    </div>
  );
}
