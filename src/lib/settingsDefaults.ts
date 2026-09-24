/**
 * The default value of every setting, in one typed object.
 *
 * It lives in a leaf module on purpose. The settings store imports `invoke`,
 * and `invoke` imports the browser-QA mock, so a mock that reached back into
 * the store for these defaults would close an import cycle and leave the
 * object undefined at module-init time, depending on which side loaded first.
 * Everything that needs the defaults imports them from here instead.
 *
 * Typed as `Settings`, so adding a setting is a type error until its default
 * is written down — which is what keeps the mock and the real backend
 * answering with the same shape.
 */
import type { Settings } from "../types";

export const DEFAULT_SETTINGS: Settings = {
  // Sending node (hsd RPC); reads come from the explorer below.
  node_rpc_url: "http://127.0.0.1:12037",
  node_rpc_api_key: "",
  hsd_prefix: "",
  hsd_path: "",
  autostart_hsd: "true",
  explorer_api_url: "https://e.hnsfans.com",
  address_gap_limit: "20",
  signer_session_timeout_seconds: "900",
  onboarding_complete: "false",
  deadline_notify_enabled: "false",
  deadline_notify_reveal_lead_blocks: "144",
  deadline_notify_renewal_lead_days: "30",
  watchlist_notify_enabled: "false",
  watchlist_notify_bidding_soon_lead_blocks: "144",
  watchlist_notify_highest_bid_threshold_hns: "",
  background_sync_enabled: "1",
  node_mode: "full",
  explorer_fallback_url: "",
  chain_source: "local_node",
  close_to_tray: "1",
  allow_remote_broadcast: "false",
  tray_hint_shown: "0",
  launch_at_login: "0",
  fee_rate_doos_per_kvb: "",
  update_notify_enabled: "false",
};
