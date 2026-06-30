import { describe, it, expect } from "vitest";
import { create } from "zustand";

describe("Settings Store Logic", () => {
  it("creates store with default values", () => {
    const store = create<{
      settings: Record<string, string> | null;
      loaded: boolean;
      passphrase: string;
    }>()((_set) => ({
      settings: null,
      loaded: false,
      passphrase: "",
    }));
    const state = store.getState();
    expect(state.settings).toBeNull();
    expect(state.loaded).toBe(false);
    expect(state.passphrase).toBe("");
  });

  it("updates settings optimistically", () => {
    const store = create<{
      settings: Record<string, string> | null;
      updateField: (key: string, value: string) => void;
    }>()((set, get) => ({
      settings: { network: "mainnet", prefix: "~/.hsd" },
      updateField: (key, value) => {
        const current = get().settings;
        if (current) set({ settings: { ...current, [key]: value } });
      },
    }));
    store.getState().updateField("network", "testnet");
    expect(store.getState().settings?.network).toBe("testnet");
    expect(store.getState().settings?.prefix).toBe("~/.hsd");
  });

  it("manages passphrase", () => {
    const store = create<{
      passphrase: string;
      setPassphrase: (p: string) => void;
      clearPassphrase: () => void;
    }>()((set) => ({
      passphrase: "",
      setPassphrase: (p) => set({ passphrase: p }),
      clearPassphrase: () => set({ passphrase: "" }),
    }));
    store.getState().setPassphrase("mypassword");
    expect(store.getState().passphrase).toBe("mypassword");
    store.getState().clearPassphrase();
    expect(store.getState().passphrase).toBe("");
  });
});

describe("UI Store Logic", () => {
  it("manages sidebar state", () => {
    const store = create<{
      sidebarCollapsed: boolean;
      toggleSidebar: () => void;
    }>()((set) => ({
      sidebarCollapsed: false,
      toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
    }));
    expect(store.getState().sidebarCollapsed).toBe(false);
    store.getState().toggleSidebar();
    expect(store.getState().sidebarCollapsed).toBe(true);
  });

  it("manages selection", () => {
    const store = create<{
      selectedAssetIds: Set<number>;
      toggleAssetSelection: (id: number) => void;
      clearSelection: () => void;
      selectAll: (ids: number[]) => void;
    }>()((set) => ({
      selectedAssetIds: new Set(),
      toggleAssetSelection: (id) =>
        set((s) => {
          const next = new Set(s.selectedAssetIds);
          if (next.has(id)) next.delete(id);
          else next.add(id);
          return { selectedAssetIds: next };
        }),
      clearSelection: () => set({ selectedAssetIds: new Set() }),
      selectAll: (ids) => set({ selectedAssetIds: new Set(ids) }),
    }));
    store.getState().toggleAssetSelection(1);
    store.getState().toggleAssetSelection(2);
    expect(store.getState().selectedAssetIds.size).toBe(2);
    store.getState().clearSelection();
    expect(store.getState().selectedAssetIds.size).toBe(0);
    store.getState().selectAll([1, 2, 3]);
    expect(store.getState().selectedAssetIds.size).toBe(3);
  });

  it("manages toast", () => {
    const store = create<{
      toastMessage: string | null;
      toastType: string;
      showToast: (msg: string, type?: string) => void;
      clearToast: () => void;
    }>()((set) => ({
      toastMessage: null,
      toastType: "info",
      showToast: (msg, type = "info") => set({ toastMessage: msg, toastType: type }),
      clearToast: () => set({ toastMessage: null }),
    }));
    store.getState().showToast("Test", "success");
    expect(store.getState().toastMessage).toBe("Test");
    store.getState().clearToast();
    expect(store.getState().toastMessage).toBeNull();
  });
});
