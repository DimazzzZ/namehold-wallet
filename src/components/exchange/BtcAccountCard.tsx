import { useState } from "react";
import { Card } from "../ui/Card";
import { CopyField } from "../ui/CopyField";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { Spinner } from "../ui/Spinner";
import { GatedButton } from "./GatedButton";
import { mapError } from "../../lib/errors";
import {
  btcText,
  parseSatsAmount,
  satsText,
  satsValue,
  UNREADABLE,
} from "../../lib/exchangeAmount";
import { exchangeTimeText } from "../../lib/exchangeStatus";
import {
  BTC_NOT_ENABLED_TEXT,
  SECURE_WINDOW_WAIT,
  WITHDRAW_ABOVE_BALANCE,
  WITHDRAW_NO_ADDRESS,
} from "../../lib/exchangeText";
import type { BtcWithdrawArgs } from "../../queries/exchange";
import type { BtcBalance } from "../../types/exchange";

function Amount({ label, sats, testId }: { label: string; sats: number; testId: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="text-gray-500">{label}</dt>
      <dd className="text-right tabular-nums" data-testid={testId}>
        <div className="text-gray-900">{btcText(sats) ?? UNREADABLE}</div>
        <div className="text-xs text-gray-500">{satsText(sats) ?? UNREADABLE}</div>
      </dd>
    </div>
  );
}

/** Each field's sentence (null = fine or empty), and the arguments once both read. */
export function checkWithdraw(
  form: { address: string; amount: string; everything: boolean },
  confirmedSats: number | null,
): { address: string | null; amount: string | null; args: BtcWithdrawArgs | null } {
  const address = form.address.trim();
  const out = {
    address: null as string | null,
    amount: null as string | null,
    args: null as BtcWithdrawArgs | null,
  };
  let sats: number | null = null;
  if (!form.everything && form.amount.trim() !== "") {
    const read = parseSatsAmount(form.amount);
    if ("error" in read) out.amount = read.error;
    else {
      const balance = confirmedSats === null ? null : satsValue(confirmedSats);
      if (balance !== null && BigInt(read.sats) > balance) {
        out.amount = WITHDRAW_ABOVE_BALANCE(satsText(balance) ?? UNREADABLE);
      } else sats = read.sats;
    }
  }
  if (address === "") return out;
  if (form.everything) out.args = { toAddress: address, amountSats: null };
  else if (sats !== null) out.args = { toAddress: address, amountSats: sats };
  return out;
}

export function BtcWithdrawDialog({
  balance,
  refusal,
  busy,
  onClose,
  onWithdraw,
}: {
  balance: BtcBalance | undefined;
  refusal: string | null;
  busy: boolean;
  onClose: () => void;
  onWithdraw: (args: BtcWithdrawArgs) => Promise<void>;
}) {
  const [form, setForm] = useState({ address: "", amount: "", everything: false });
  const [touched, setTouched] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const check = checkWithdraw(form, balance?.confirmedSats ?? null);
  const set = (patch: Partial<typeof form>) => {
    setForm((f) => ({ ...f, ...patch }));
    setError(null);
  };
  const submit = async () => {
    setTouched(true);
    if (!check.args) return;
    try {
      await onWithdraw(check.args);
    } catch (e) {
      setError(mapError(e));
    }
  };
  const addressError = touched && form.address.trim() === "" ? WITHDRAW_NO_ADDRESS : null;

  return (
    <Dialog open onClose={() => !busy && onClose()} title="Withdraw BTC" className="max-w-[460px]">
      <div className="space-y-3 text-sm">
        <div>
          <Input
            label="Bitcoin address"
            id="btc-withdraw-address"
            data-testid="btc-withdraw-address"
            autoComplete="off"
            spellCheck={false}
            value={form.address}
            onChange={(e) => set({ address: e.target.value })}
          />
          {addressError && (
            <p role="alert" className="mt-1 text-xs text-red-600" data-testid="btc-address-error">
              {addressError}
            </p>
          )}
        </div>
        <div>
          <Input
            label="Amount in sats"
            id="btc-withdraw-amount"
            data-testid="btc-withdraw-amount"
            inputMode="numeric"
            autoComplete="off"
            disabled={form.everything}
            value={form.everything ? "" : form.amount}
            onChange={(e) => set({ amount: e.target.value })}
          />
          {check.amount && (
            <p role="alert" className="mt-1 text-xs text-red-600" data-testid="btc-amount-error">
              {check.amount}
            </p>
          )}
          <label className="mt-2 flex items-center gap-2 text-gray-700">
            <input
              type="checkbox"
              data-testid="btc-withdraw-everything"
              checked={form.everything}
              onChange={(e) => set({ everything: e.target.checked })}
            />
            Send everything
          </label>
        </div>
        {error && (
          <p role="alert" className="text-xs text-red-600" data-testid="btc-withdraw-error">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2 pt-1">
          <Button
            variant="ghost"
            data-testid="btc-withdraw-cancel"
            onClick={onClose}
            disabled={busy}
          >
            Cancel
          </Button>
          <GatedButton
            variant="primary"
            data-testid="btc-withdraw-submit"
            refusal={refusal}
            disabled={check.args === null && form.address.trim() !== ""}
            loading={busy}
            onClick={() => void submit()}
          >
            {busy ? SECURE_WINDOW_WAIT : "Withdraw"}
          </GatedButton>
        </div>
      </div>
    </Dialog>
  );
}

/** The profile's BIP 84 Bitcoin account: balance, receive address, Withdraw. */
export function BtcAccountCard({
  enabled,
  balance,
  loading,
  error,
  refusal,
  onWithdraw,
}: {
  enabled: boolean;
  balance: BtcBalance | undefined;
  loading: boolean;
  error: unknown;
  refusal: string | null;
  onWithdraw: () => void;
}) {
  const checked = balance ? exchangeTimeText(balance.checkedAt) : null;
  return (
    <Card
      title="BTC account"
      subtitle={checked ? `Checked ${checked}` : undefined}
      actions={
        <GatedButton
          size="sm"
          variant="secondary"
          data-testid="btc-withdraw-open"
          refusal={refusal}
          onClick={onWithdraw}
        >
          Withdraw…
        </GatedButton>
      }
    >
      <div data-testid="btc-account" className="space-y-3 text-sm">
        {!enabled ? (
          <p className="text-gray-500">{BTC_NOT_ENABLED_TEXT}</p>
        ) : loading ? (
          <div className="flex items-center gap-2 text-gray-500">
            <Spinner /> Reading the BTC account…
          </div>
        ) : error != null ? (
          <p role="alert" className="text-red-600" data-testid="btc-balance-error">
            {mapError(error)}
          </p>
        ) : balance ? (
          <>
            <dl className="space-y-2">
              <Amount label="Confirmed" sats={balance.confirmedSats} testId="btc-confirmed" />
              <Amount label="Unconfirmed" sats={balance.unconfirmedSats} testId="btc-unconfirmed" />
              <Amount
                label="Locked in trades"
                sats={balance.lockedInTradesSats}
                testId="btc-locked"
              />
            </dl>
            {balance.error && (
              <p role="alert" className="text-xs text-red-700" data-testid="btc-source-error">
                {balance.error}
              </p>
            )}
            <CopyField
              label="Receive address"
              value={balance.receiveAddress}
              layout="stacked"
              toastLabel="Address"
              valueTestId="btc-receive-address"
              copyTestId="btc-receive-copy"
            />
          </>
        ) : null}
      </div>
    </Card>
  );
}
