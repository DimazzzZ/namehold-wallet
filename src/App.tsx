import { BrowserRouter, Routes, Route } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect } from "react";
import { Layout } from "./components/Layout";
import { Dashboard } from "./components/Dashboard";
import { TldInventory } from "./components/TldInventory";
import { Batches } from "./components/Batches";
import { WalletView } from "./components/WalletView";
import { SyncVerification } from "./components/SyncVerification";
import { Renewals } from "./components/Renewals";
import { DnsRecords } from "./components/DnsRecords";
import { Settings } from "./components/Settings";
import { useSettingsStore } from "./stores/settings";
import "./app.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 10_000,
      refetchOnWindowFocus: false,
    },
  },
});

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
        <Routes>
          <Route element={<Layout />}>
            <Route path="/" element={<Dashboard />} />
            <Route path="/inventory" element={<TldInventory />} />
            <Route path="/batches" element={<Batches />} />
            <Route path="/wallet" element={<WalletView />} />
            <Route path="/sync" element={<SyncVerification />} />
            <Route path="/renewals" element={<Renewals />} />
            <Route path="/dns" element={<DnsRecords />} />
            <Route path="/settings" element={<Settings />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </QueryClientProvider>
  );
}
