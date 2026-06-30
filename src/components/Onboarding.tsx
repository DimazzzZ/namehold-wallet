import { useQueryClient } from "@tanstack/react-query";
import { useSettingsStore } from "../stores/settings";
import { AddWalletForm } from "./AddWalletForm";

/**
 * Wallet-first, non-custodial onboarding (first run, zero profiles).
 *
 * Secrets never touch React: creating or importing a wallet hands off to the
 * Rust-owned secure window. This screen is just the welcome chrome around the
 * shared `AddWalletForm`; on success it marks onboarding complete.
 */
export function Onboarding() {
  const qc = useQueryClient();
  const saveAll = useSettingsStore((s) => s.saveAll);

  const finish = async () => {
    await saveAll({ onboarding_complete: "true" });
    qc.invalidateQueries({ queryKey: ["wallet"] });
    qc.invalidateQueries({ queryKey: ["read"] });
  };

  return (
    <div className="flex h-screen items-center justify-center bg-gray-100 p-6">
      <div className="bg-white rounded-lg shadow-lg max-w-lg w-full p-8">
        <h1 className="text-2xl font-bold text-gray-900 mb-2">Welcome to Namehold</h1>
        <p className="text-gray-500 mb-6">
          A non-custodial wallet for moving and managing Handshake names. Your keys never
          leave this device, and your recovery phrase is only ever shown in a secure window.
        </p>
        <AddWalletForm defaultLabel="Primary" onDone={finish} />
      </div>
    </div>
  );
}
