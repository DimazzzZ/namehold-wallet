import { Button } from "./ui/Button";
import {
  useAddToWatchlist,
  useRemoveFromWatchlist,
  useWatchlistMembership,
} from "../queries/watchlist";

export interface WatchlistToggleProps {
  name: string;
  /** Optional compact styling for placement in a modal header. */
  size?: "sm" | "md";
}

/**
 * A small toggle button that adds/removes a single name from the watchlist.
 * Reads the current state via `get_watchlist_status` for just this name, so
 * the label reflects whether it's already tracked.
 */
export function WatchlistToggle({ name, size = "sm" }: WatchlistToggleProps) {
  const { data: status } = useWatchlistMembership(name);
  const add = useAddToWatchlist();
  const remove = useRemoveFromWatchlist();
  const watched = status?.watched ?? false;
  const pending = add.isPending || remove.isPending;

  return (
    <Button
      size={size}
      variant={watched ? "ghost" : "primary"}
      onClick={() => (watched ? remove.mutate(name) : add.mutate(name))}
      disabled={pending}
      data-testid="watchlist-toggle"
    >
      {watched ? "Remove from Watchlist" : "Add to Watchlist"}
    </Button>
  );
}
