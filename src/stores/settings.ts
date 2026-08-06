import { create } from "zustand";
import { invoke } from "../lib/invoke";
import type { Settings } from "../types";

interface SettingsState {
  settings: Settings | null;
  loaded: boolean;
  load: () => Promise<void>;
  update: (key: string, value: string) => Promise<void>;
  saveAll: (partial: Partial<Settings>) => Promise<void>;
}

const DEFAULT_SETTINGS: Settings = {
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
};

export const useSettingsStore = create<SettingsState>((set, get) => ({
  settings: null,
  loaded: false,
  load: async () => {
    const s = await invoke<Record<string, string>>("get_settings");
    set({ settings: { ...DEFAULT_SETTINGS, ...s }, loaded: true });
  },
  saveAll: async (partial: Partial<Settings>) => {
    const current = get().settings;
    if (!current) return;
    const merged = { ...current, ...partial };
    set({ settings: merged });
    for (const [key, value] of Object.entries(partial)) {
      await invoke("update_setting", { key, value: String(value) });
    }
  },
  update: async (key, value) => {
    const current = get().settings;
    if (current) {
      set({ settings: { ...current, [key]: value } });
    }
    await invoke("update_setting", { key, value });
  },
}));
