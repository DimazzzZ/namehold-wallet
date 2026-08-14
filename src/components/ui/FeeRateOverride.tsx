import { useState } from "react";
import { Input } from "./Input";
import {
  MIN_FEE_RATE_DOOS_PER_KVB,
  parseDoosPerKvb,
} from "../../lib/feeRate";

export interface FeeRateOverrideProps {
  /** Current raw text (kept in the parent so submit paths can read it). */
  value: string;
  onChange: (raw: string) => void;
  /** Optional label prefix; defaults to "Advanced". */
  label?: string;
  /**
   * If `true`, the disclosure starts open (e.g. when re-opening a modal that
   * previously carried a non-empty override).
   */
  defaultOpen?: boolean;
  /**
   * Text shown after the numeric input, to remind the user what the setting
   * default is. Optional — the parent already knows the setting.
   */
  settingDefaultHint?: string;
}

/**
 * Advanced-disclosure fee-rate override input, in doos/kvB (matches the
 * setting unit). The parent owns the raw string; call
 * `doosPerKvbToSatsPerByte(parseDoosPerKvb(raw))` at submit time and pass the
 * result as the draft-builder's `feeRate` argument.
 *
 * Empty input means "no override — fall through to the setting default";
 * non-empty must parse to a positive integer (floored to the minimum).
 */
export function FeeRateOverride({
  value,
  onChange,
  label = "Advanced",
  defaultOpen = false,
  settingDefaultHint,
}: FeeRateOverrideProps) {
  const [open, setOpen] = useState(defaultOpen || value.trim().length > 0);
  const trimmed = value.trim();
  const error =
    trimmed && parseDoosPerKvb(trimmed) === null
      ? "Fee rate must be a whole number of doos/kvB"
      : null;

  return (
    <div className="border border-gray-200 rounded">
      <button
        type="button"
        className="w-full text-left px-3 py-2 hover:bg-gray-50 flex items-center justify-between text-xs"
        onClick={() => setOpen((prev) => !prev)}
        data-testid="fee-rate-override-toggle"
      >
        <span className="font-medium text-gray-700">
          {open ? "\u25BC" : "\u25B6"} {label}
        </span>
        {trimmed && !open ? (
          <span className="text-gray-500">
            fee rate: {trimmed} doos/kvB
          </span>
        ) : null}
      </button>
      {open ? (
        <div className="px-3 py-3 border-t border-gray-200 bg-gray-50 space-y-2">
          <Input
            label="Fee rate (doos/kvB)"
            value={value}
            onChange={(e) => onChange(e.target.value)}
            placeholder={
              settingDefaultHint ??
              `empty = use setting default (min ${MIN_FEE_RATE_DOOS_PER_KVB.toLocaleString()})`
            }
            inputMode="numeric"
            data-testid="fee-rate-override-input"
          />
          {error ? (
            <div className="text-xs text-red-600">{error}</div>
          ) : (
            <div className="text-xs text-gray-500">
              Applies to this transaction only. 1000 doos/kvB = 1 sat/byte.
            </div>
          )}
        </div>
      ) : null}
    </div>
  );
}

/** True if the raw value is empty OR parses to a valid rate. */
export function feeRateOverrideIsValid(raw: string): boolean {
  const trimmed = raw.trim();
  return trimmed.length === 0 || parseDoosPerKvb(trimmed) !== null;
}
