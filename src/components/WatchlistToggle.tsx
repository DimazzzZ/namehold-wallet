import { Star } from "lucide-react";
import { Button } from "./ui/Button";
import { Icon } from "./ui/Icon";
import { Tooltip } from "./ui/Tooltip";
import { cn } from "../lib/utils";
import {
  useAddToWatchlist,
  useRemoveFromWatchlist,
  useWatchlistMembership,
} from "../queries/watchlist";

/**
 * Watch / Watching toggle for one name, in the name page header. Reads the
 * current state via `get_watchlist_status` for just this name.
 */
export function WatchlistToggle({ name }: { name: string }) {
  const { data: status } = useWatchlistMembership(name);
  const add = useAddToWatchlist();
  const remove = useRemoveFromWatchlist();
  const watched = status?.watched ?? false;
  const pending = add.isPending || remove.isPending;

  return (
    <Tooltip content={watched ? "Remove from Watchlist" : "Add to Watchlist"}>
      <Button
        variant="secondary"
        onClick={() => (watched ? remove.mutate(name) : add.mutate(name))}
        disabled={pending}
        aria-pressed={watched}
        data-testid="watchlist-toggle"
      >
        <Icon
          icon={Star}
          size={14}
          className={cn(watched ? "fill-amber-500 text-amber-500" : "text-gray-500")}
        />
        {watched ? "Watching" : "Watch"}
      </Button>
    </Tooltip>
  );
}
