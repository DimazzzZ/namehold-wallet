import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useNavigate } from "react-router-dom";
import { isTauri } from "../lib/runtime";

/** Wallet reads this and starts Renew all; see `RENEW_ALL_EVENT` in deadlines.rs. */
export const RENEW_ALL_PARAM = "renewAll";

/** A press on a renewal notification opens the Wallet page, which asks to confirm. */
export function useRenewAllRequest() {
  const navigate = useNavigate();
  useEffect(() => {
    if (!isTauri()) return;
    const stop = listen("renew-all-requested", () => navigate(`/?${RENEW_ALL_PARAM}=1`));
    return () => void stop.then((f) => f());
  }, [navigate]);
}
