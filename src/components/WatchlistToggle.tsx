import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../lib/invoke";
import { Button } from "./ui/Button";
import { useUiStore } from "../stores/ui";

interface WatchlistStatus {
  name: string;
  watched: boolean;
  tags: string;
  state: string | null;
  expiry: number | null;
}

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
  const qc = useQueryClient();
  const showToast = useUiStore((s) => s.showToast);

  const { data: status } = useQuery<WatchlistStatus | undefined>({
    // Distinct from Watchlist's bulk `["watchlist","status", <joined names>]`
    // key — same shape but the queryFn here returns a single row (`rows[0]`),
    // so sharing the key with the bulk query would corrupt the cached shape
    // when a user watches exactly one name and opens its info modal.
    queryKey: ["watchlist", "toggle", name],
    queryFn: async () => {
      const rows = await invoke<WatchlistStatus[]>("get_watchlist_status", {
        names: [name],
      });
      return rows[0];
    },
  });

  const watched = status?.watched ?? false;

  const toggleMutation = useMutation({
    mutationFn: () =>
      watched
        ? invoke("remove_from_watchlist", { name })
        : invoke("add_to_watchlist", { name }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["watchlist"] });
      showToast(
        watched ? `Removed ${name} from watchlist` : `Added ${name} to watchlist`,
        watched ? "info" : "success",
      );
    },
    onError: (err) => {
      showToast(`Watchlist update failed: ${err}`, "error");
    },
  });

  return (
    <Button
      size={size}
      variant={watched ? "ghost" : "primary"}
      onClick={() => toggleMutation.mutate()}
      disabled={toggleMutation.isPending}
      data-testid="watchlist-toggle"
    >
      {watched ? "Remove from Watchlist" : "Add to Watchlist"}
    </Button>
  );
}
