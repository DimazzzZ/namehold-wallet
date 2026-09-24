import { create } from "zustand";
import { invoke } from "../lib/invoke";
import { DEFAULT_SETTINGS } from "../lib/settingsDefaults";
import type { Settings } from "../types";

interface SettingsState {
  settings: Settings | null;
  loaded: boolean;
  load: () => Promise<void>;
  update: (key: string, value: string) => Promise<void>;
  saveAll: (partial: Partial<Settings>) => Promise<void>;
}

export { DEFAULT_SETTINGS } from "../lib/settingsDefaults";

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
