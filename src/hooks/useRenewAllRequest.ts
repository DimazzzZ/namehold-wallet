import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useNavigate } from "react-router-dom";
import { isTauri } from "../lib/runtime";

/** Holds the wallet a reminder is for; see `RENEW_ALL_EVENT` in deadlines.rs. */
export const RENEW_ALL_PARAM = "renewAll";

/** A press on a renewal notification opens the Wallet page, which asks to confirm. */
export function useRenewAllRequest() {
  const navigate = useNavigate();
  useEffect(() => {
    if (!isTauri()) return;
    const stop = listen<string>("renew-all-requested", ({ payload }) =>
      navigate(`/?${RENEW_ALL_PARAM}=${encodeURIComponent(payload)}`),
    );
    return () => void stop.then((f) => f());
  }, [navigate]);
}
