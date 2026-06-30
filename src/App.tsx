import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect } from "react";
import { Layout } from "./components/Layout";
import { Overview } from "./components/Overview";
import { PortfolioWorkspace } from "./components/PortfolioWorkspace";
import { MigrationWorkspace } from "./components/MigrationWorkspace";
import { WalletView } from "./components/WalletView";
import { NodeControl } from "./components/NodeControl";
import { Settings } from "./components/Settings";
import { Onboarding } from "./components/Onboarding";
import { useSettingsStore } from "./stores/settings";
import { useWalletList } from "./queries/wallet";
import "./app.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 10_000,
      refetchOnWindowFocus: false,
    },
  },
});

function AppRoutes() {
  const settings = useSettingsStore((s) => s.settings);
  const { data: walletList } = useWalletList();
  const currentWalletId = settings?.hsd_wallet_id || "";

  const hasWallets = walletList && walletList.length > 0;
  const hasSelectedWallet = currentWalletId.trim().length > 0;

  if (!hasSelectedWallet && !hasWallets) {
    return <Onboarding />;
  }

  return (
    <Routes>
      <Route element={<Layout />}>
        <Route path="/" element={<Overview />} />
        <Route path="/portfolio" element={<PortfolioWorkspace />} />
        <Route path="/migration" element={<MigrationWorkspace />} />
        <Route path="/wallet" element={<WalletView />} />
        <Route path="/node" element={<NodeControl />} />
        <Route path="/settings" element={<Settings />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Route>
    </Routes>
  );
}

export default function App() {
  const loadSettings = useSettingsStore((s) => s.load);
  const loaded = useSettingsStore((s) => s.loaded);

  useEffect(() => {
    loadSettings();
  }, [loadSettings]);

  if (!loaded) {
    return (
      <div className="flex h-screen items-center justify-center bg-gray-100">
        <div className="text-gray-500">Loading...</div>
      </div>
    );
  }

  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <AppRoutes />
      </BrowserRouter>
    </QueryClientProvider>
  );
}
