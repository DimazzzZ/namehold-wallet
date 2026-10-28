import { useNavigate } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import { namePath, type NameTab } from "../lib/nameRoute";
import { nameCapabilitiesQueryKey } from "../queries/read";
import { useActiveProfile } from "../queries/wallet";
import type { NameActionCapabilities } from "../types";

/**
 * The one way any screen opens a name: its page. With `seedCaps` (a row's
 * batch capabilities), the page's single-name query starts from them, so it
 * opens on the row's status with no flash of the raw phase.
 */
export function useOpenName() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const profileId = useActiveProfile().data?.id ?? null;
  return (rawName: string, opts?: { tab?: NameTab; seedCaps?: NameActionCapabilities }) => {
    if (opts?.seedCaps) {
      qc.setQueryData(nameCapabilitiesQueryKey(profileId, rawName), opts.seedCaps);
    }
    navigate(namePath(rawName, opts?.tab));
  };
}
