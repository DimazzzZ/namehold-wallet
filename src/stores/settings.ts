import { create } from "zustand";
import { invoke } from "../lib/invoke";
import type { Settings } from "../types";

interface SettingsState {
  settings: Settings | null;
  loaded: boolean;
  passphrase: string;
  load: () => Promise<void>;
  update: (key: string, value: string) => Promise<void>;
  saveAll: (partial: Partial<Settings>) => Promise<void>;
  setPassphrase: (p: string) => void;
  clearPassphrase: () => void;
}

const DEFAULT_SETTINGS: Settings = {
  hsd_wallet_api_url: "http://127.0.0.1:12039",
  hsd_node_api_url: "http://127.0.0.1:12037",
  hsd_api_key: "",
  hsd_wallet_id: "primary",
  hsd_network: "mainnet",
  hsd_prefix: "~/.hsd",
  write_mode: "false",
  connection_mode: "local_managed_hsd",
  external_read_provider: "none",
  external_read_api_url: "https://hnsfans.com",
  external_read_watch_addresses: "[]",
  external_read_watch_names: "[]",
  remote_hsd_label: "",
  trusted_remote_hsd: "false",
  future_signer_mode: "none",
  advanced_mode: "false",
  onboarding_complete: "false",
};

export const useSettingsStore = create<SettingsState>((set, get) => ({
  settings: null,
  loaded: false,
  passphrase: "",
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
  setPassphrase: (p) => set({ passphrase: p }),
  clearPassphrase: () => set({ passphrase: "" }),
}));
