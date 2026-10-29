import { useNeedsYou } from "../queries/needsYou";
import { useToFinishCount } from "../queries/activity";

/** The amber counts beside Names and Activity in the nav; 0 hides them. */
export function useNavCounts(): { names: number; activity: number } {
  const needsYou = useNeedsYou();
  const activity = useToFinishCount();
  return { names: needsYou.isLoading ? 0 : needsYou.items.length, activity };
}
