import { create } from "zustand";
import { invoke } from "../lib/invoke";
import type { Settings } from "../types";

interface SettingsState {
  settings: Settings | null;
  loaded: boolean;
  passphrase: string;
  load: () => Promise<void>;
  update: (key: string, value: string) => Promise<void>;
  setPassphrase: (p: string) => void;
  clearPassphrase: () => void;
}

const DEFAULT_SETTINGS: Settings = {
  hsd_wallet_api_url: "http://127.0.0.1:12039",
  hsd_node_api_url: "http://127.0.0.1:12037",
  hsd_api_key: "",
  hsd_wallet_id: "primary",
  hsd_network: "mainnet",
  write_mode: "false",
};

export const useSettingsStore = create<SettingsState>((set, get) => ({
  settings: null,
  loaded: false,
  passphrase: "",
  load: async () => {
    const s = await invoke<Record<string, string>>("get_settings");
    set({ settings: { ...DEFAULT_SETTINGS, ...s }, loaded: true });
  },
  update: async (key, value) => {
    await invoke("update_setting", { key, value });
    const current = get().settings;
    if (current) {
      set({ settings: { ...current, [key]: value } });
    }
  },
  setPassphrase: (p) => set({ passphrase: p }),
  clearPassphrase: () => set({ passphrase: "" }),
}));
