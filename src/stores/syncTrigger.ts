import { create } from "zustand";

interface SyncTriggerState {
  /** Whether the current sync was triggered manually (vs automatic). */
  manualSync: boolean;
  setManualSync: (manual: boolean) => void;
}

export const useSyncTriggerStore = create<SyncTriggerState>((set) => ({
  manualSync: false,
  setManualSync: (manual) => set({ manualSync: manual }),
}));
