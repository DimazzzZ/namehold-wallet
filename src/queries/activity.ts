import { useMemo } from "react";
import { isToFinish, mergeActivity, type MergedRow } from "../lib/activity";
import { useActionHistory } from "./read";
import { useActiveProfile, useTxDrafts } from "./wallet";

/** Chain history merged with local drafts — the one list every activity surface reads. */
export function useMergedActivity(): {
  rows: MergedRow[];
  isLoading: boolean;
  isFetching: boolean;
  isError: boolean;
  error: unknown;
} {
  const history = useActionHistory();
  const { data: drafts } = useTxDrafts();
  const { data: profile } = useActiveProfile();
  const profileId = profile?.id ?? null;
  // `list_tx_drafts` answers for the active wallet, but its cache key carries
  // no profile: right after a switch the cached list can still be the other
  // wallet's. Keep only this wallet's drafts so the page, the Wallet card and
  // the nav count never describe another wallet.
  const rows = useMemo(
    () =>
      mergeActivity(
        history.data ?? [],
        (drafts ?? []).filter((d) => d.walletProfileId === profileId),
      ),
    [history.data, drafts, profileId],
  );
  return {
    rows,
    isLoading: history.isLoading,
    isFetching: history.isFetching,
    isError: history.isError,
    error: history.error,
  };
}

/** Rows in Activity › To finish, for the sidebar count. 0 while drafts load. */
export function useToFinishCount(): number {
  return useMergedActivity().rows.filter(isToFinish).length;
}
