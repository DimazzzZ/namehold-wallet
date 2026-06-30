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
  explorer_api_url: "https://e.hnsfans.com",
  address_gap_limit: "20",
  signer_session_timeout_seconds: "900",
  advanced_mode: "false",
  onboarding_complete: "false",
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
