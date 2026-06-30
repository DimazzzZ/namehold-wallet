import { create } from "zustand";
import type { PortfolioSectionKey, MigrationSectionKey } from "../types";

export type ToastType = "info" | "error" | "success";

export interface ToastEntry {
  id: string;
  message: string;
  type: ToastType;
}

interface UiState {
  sidebarCollapsed: boolean;
  toggleSidebar: () => void;
  selectedAssetIds: Set<number>;
  toggleAssetSelection: (id: number) => void;
  clearSelection: () => void;
  selectAll: (ids: number[]) => void;
  // Workspace tab state
  activePortfolioTab: PortfolioSectionKey;
  setActivePortfolioTab: (tab: PortfolioSectionKey) => void;
  activeMigrationTab: MigrationSectionKey;
  setActiveMigrationTab: (tab: MigrationSectionKey) => void;
  // Toast queue
  toastQueue: ToastEntry[];
  dismissToast: (id: string) => void;
  // Backward-compatible single-toast accessors (derived from the queue head)
  toastMessage: string | null;
  toastType: ToastType;
  showToast: (message: string, type?: ToastType) => void;
  clearToast: () => void;
}

let toastCounter = 0;
function nextToastId(): string {
  toastCounter += 1;
  return `toast-${Date.now()}-${toastCounter}`;
}

export const useUiStore = create<UiState>((set, get) => ({
  sidebarCollapsed: false,
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  selectedAssetIds: new Set(),
  toggleAssetSelection: (id) =>
    set((s) => {
      const next = new Set(s.selectedAssetIds);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return { selectedAssetIds: next };
    }),
  clearSelection: () => set({ selectedAssetIds: new Set() }),
  selectAll: (ids) => set({ selectedAssetIds: new Set(ids) }),

  activePortfolioTab: "inventory",
  setActivePortfolioTab: (tab) => set({ activePortfolioTab: tab }),
  activeMigrationTab: "namebase",
  setActiveMigrationTab: (tab) => set({ activeMigrationTab: tab }),

  toastQueue: [],
  dismissToast: (id) =>
    set((s) => {
      const remaining = s.toastQueue.filter((t) => t.id !== id);
      const head = remaining[0] ?? null;
      return {
        toastQueue: remaining,
        toastMessage: head?.message ?? null,
        toastType: head?.type ?? "info",
      };
    }),

  toastMessage: null,
  toastType: "info",
  showToast: (message, type = "info") => {
    const id = nextToastId();
    set((s) => ({
      toastQueue: [...s.toastQueue, { id, message, type }],
      toastMessage: message,
      toastType: type,
    }));
    setTimeout(() => {
      get().dismissToast(id);
    }, 4000);
  },
  clearToast: () => set({ toastQueue: [], toastMessage: null }),
}));
