import { create } from "zustand";
import { invoke } from "../lib/invoke";
import { DEFAULT_SETTINGS } from "../lib/settingsDefaults";
import {
  isConfirmationDeclined,
  isProtectedSettingKey,
  type SettingChange,
} from "../lib/protectedSettings";
import type { Settings } from "../types";

interface SettingsState {
  settings: Settings | null;
  loaded: boolean;
  load: () => Promise<void>;
  update: (key: string, value: string) => Promise<void>;
  /**
   * Save `partial`. Protected settings (lib/protectedSettings.ts) that differ
   * from the stored ones go first, in one `update_protected_settings` call
   * that the user confirms in the secure window; the rest are written with
   * `update_setting` after it. Resolves `false`, with nothing saved, when the
   * user declines that confirmation; any other failure rejects.
   */
  saveAll: (partial: Partial<Settings> | Record<string, string>) => Promise<boolean>;
}

export { DEFAULT_SETTINGS } from "../lib/settingsDefaults";

export const useSettingsStore = create<SettingsState>((set, get) => ({
  settings: null,
  loaded: false,
  load: async () => {
    const s = await invoke<Record<string, string>>("get_settings");
    set({ settings: { ...DEFAULT_SETTINGS, ...s }, loaded: true });
  },
  saveAll: async (partial) => {
    const current = get().settings;
    if (!current) return false;
    const stored = current as unknown as Record<string, string | undefined>;
    const changes: SettingChange[] = [];
    const plain: [string, string][] = [];
    for (const [key, value] of Object.entries(partial)) {
      const text = String(value);
      if (!isProtectedSettingKey(key)) {
        plain.push([key, text]);
      } else if (text !== (stored[key] ?? "")) {
        changes.push({ key, value: text });
      }
    }
    if (changes.length > 0) {
      try {
        await invoke("update_protected_settings", { changes });
      } catch (e) {
        if (isConfirmationDeclined(e)) return false;
        throw e;
      }
    }
    set({ settings: { ...current, ...partial } as Settings });
    for (const [key, value] of plain) {
      await invoke("update_setting", { key, value });
    }
    return true;
  },
  update: async (key, value) => {
    const current = get().settings;
    if (current) {
      set({ settings: { ...current, [key]: value } });
    }
    await invoke("update_setting", { key, value });
  },
}));
