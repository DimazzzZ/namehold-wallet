import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect } from "react";
import { Layout } from "./components/Layout";
import { PortfolioWorkspace } from "./components/PortfolioWorkspace";
import { MigrationWorkspace } from "./components/MigrationWorkspace";
import { WalletView } from "./components/WalletView";
import { Settings } from "./components/Settings";
import { Onboarding } from "./components/Onboarding";
import { useSettingsStore } from "./stores/settings";
import { useWalletProfiles } from "./queries/wallet";
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
  const { data: profiles } = useWalletProfiles();

  // Onboarding shows until a non-custodial wallet profile exists (or the user
  // explicitly finished onboarding).
  const hasProfile = (profiles?.length ?? 0) > 0;
  const onboardingComplete = settings?.onboarding_complete === "true";

  if (!onboardingComplete && !hasProfile) {
    return <Onboarding />;
  }

  return (
    <Routes>
      <Route element={<Layout />}>
        {/* Wallet-first: the wallet is the default landing screen. */}
        <Route path="/" element={<WalletView />} />
        <Route path="/migration" element={<MigrationWorkspace />} />
        <Route path="/portfolio" element={<PortfolioWorkspace />} />
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
