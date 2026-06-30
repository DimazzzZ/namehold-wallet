import { create } from "zustand";

interface UiState {
  sidebarCollapsed: boolean;
  toggleSidebar: () => void;
  selectedAssetIds: Set<number>;
  toggleAssetSelection: (id: number) => void;
  clearSelection: () => void;
  selectAll: (ids: number[]) => void;
  toastMessage: string | null;
  toastType: "info" | "error" | "success";
  showToast: (message: string, type?: "info" | "error" | "success") => void;
  clearToast: () => void;
}

export const useUiStore = create<UiState>((set) => ({
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
  toastMessage: null,
  toastType: "info",
  showToast: (message, type = "info") => {
    set({ toastMessage: message, toastType: type });
    setTimeout(() => {
      set((s) => {
        if (s.toastMessage === message) {
          return { toastMessage: null };
        }
        return {};
      });
    }, 4000);
  },
  clearToast: () => set({ toastMessage: null }),
}));
