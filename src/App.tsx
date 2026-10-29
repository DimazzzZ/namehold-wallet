import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { ROUTE_ALIASES } from "./lib/navigation";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect } from "react";
import { Layout } from "./components/Layout";
import { AboutPage } from "./components/AboutPage";
import { WalletView } from "./components/WalletView";
import { ActivityView } from "./components/ActivityView";
import MarketPage from "./components/market/MarketPage";
import { NamesView } from "./components/NamesView";
import { SettingsInterim } from "./components/interim/SettingsInterim";
import { Onboarding } from "./components/Onboarding";
import { useSettingsStore } from "./stores/settings";
import { useWalletProfiles, useDraftConfirmationWatcher } from "./queries/wallet";
import { useAutoSync } from "./queries/autoSync";
import { useLockDiscovery } from "./queries/lockDiscovery";
import { settingToBool } from "./lib/settingsBool";
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
  // Watch drafts polling for UPDATE/REGISTER confirmations to invalidate the
  // records read + run best-effort write-back verification (Follow-up 3).
  // Mounted here so the watcher fires regardless of which route is active.
  useDraftConfirmationWatcher();
  // Keep cached data (balances, owned names, transactions) fresh from the node
  // automatically while it's live — no manual Refresh needed.
  useAutoSync();
  // Find the profile's own Shakedex locks (R34) after a sync, once it is unlocked.
  useLockDiscovery();

  // Onboarding shows until a non-custodial wallet profile exists (or the user
  // explicitly finished onboarding).
  const hasProfile = (profiles?.length ?? 0) > 0;
  const onboardingComplete = settingToBool(settings?.onboarding_complete);

  if (!onboardingComplete && !hasProfile) {
    return <Onboarding />;
  }

  return (
    <Routes>
      <Route element={<Layout />}>
        {/* Wallet-first: the wallet is the default landing screen. */}
        <Route path="/" element={<WalletView />} />
        <Route path="/names" element={<NamesView />} />
        <Route path="/market" element={<MarketPage />} />
        <Route path="/activity" element={<ActivityView />} />
        <Route path="/settings" element={<SettingsInterim />} />
        <Route path="/about" element={<AboutPage />} />
        {ROUTE_ALIASES.map((a) => (
          <Route key={a.from} path={a.from} element={<Navigate to={a.to} replace />} />
        ))}
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
        <div className="flex flex-col items-center gap-4">
          {/* Spinner (CSS, matching Button.tsx pattern) */}
          <span className="inline-block h-8 w-8 animate-spin rounded-full border-4 border-gray-300 border-t-blue-600" />
          {/* App name */}
          <div className="text-lg font-semibold text-gray-700">Namehold</div>
          {/* Status message */}
          <div className="text-sm text-gray-500">Starting up…</div>
        </div>
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
