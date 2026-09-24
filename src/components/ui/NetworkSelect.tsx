import type { WalletNetwork } from "../../types";

export interface NetworkSelectProps {
  /** The currently chosen network. */
  value: WalletNetwork;
  /** Called with the new network. */
  onChange: (network: WalletNetwork) => void;
  /** testid for the select. */
  testId?: string;
  /** Extra classes on the select, for a caller that constrains its width. */
  className?: string;
}

/**
 * The network picker, shared by first-run onboarding and Add wallet.
 *
 * A profile's network is immutable once created, so this is the last moment
 * the choice can be made and the one place the three options are listed. The
 * two screens each had their own copy and had already drifted: only one
 * carried a `data-testid`, so only one was reachable from a test.
 *
 * Which networks exist is a fact about the wallet, not about either screen.
 */
export function NetworkSelect({ value, onChange, testId, className }: NetworkSelectProps) {
  return (
    <select
      className={`border border-gray-300 rounded px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500${
        className ? ` ${className}` : ""
      }`}
      value={value}
      onChange={(e) => onChange(e.target.value as WalletNetwork)}
      data-testid={testId}
    >
      <option value="mainnet">Mainnet</option>
      <option value="testnet">Testnet</option>
      <option value="regtest">Regtest (local testing only)</option>
    </select>
  );
}
