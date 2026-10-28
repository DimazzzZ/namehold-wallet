import { create } from "zustand";
import type { MigrationSectionKey } from "../types";

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
  activeMigrationTab: MigrationSectionKey;
  setActiveMigrationTab: (tab: MigrationSectionKey) => void;
  // Shell v2: one Wallets dialog for the whole app, and the two shell
  // popovers (opening one closes the other).
  walletsDialog: null | "list" | "add";
  openWallets: (mode: "list" | "add") => void;
  closeWallets: () => void;
  walletMenuOpen: boolean;
  setWalletMenuOpen: (open: boolean) => void;
  readinessOpen: boolean;
  setReadinessOpen: (open: boolean) => void;
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

  activeMigrationTab: "namebase",
  setActiveMigrationTab: (tab) => set({ activeMigrationTab: tab }),

  walletsDialog: null,
  openWallets: (mode) => set({ walletsDialog: mode, walletMenuOpen: false, readinessOpen: false }),
  closeWallets: () => set({ walletsDialog: null }),
  walletMenuOpen: false,
  setWalletMenuOpen: (open) =>
    set(open ? { walletMenuOpen: true, readinessOpen: false } : { walletMenuOpen: false }),
  readinessOpen: false,
  setReadinessOpen: (open) =>
    set(open ? { readinessOpen: true, walletMenuOpen: false } : { readinessOpen: false }),

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
