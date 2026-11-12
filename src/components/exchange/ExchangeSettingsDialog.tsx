import { useState } from "react";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { GatedButton } from "./GatedButton";
import { mapError } from "../../lib/errors";
import {
  AUTH_CLEARED,
  AUTH_INVALID,
  AUTH_SET,
  NOSTR_HINT,
  NOSTR_INVALID,
  RELAY_HINT,
  SECURE_WINDOW_WAIT,
  SOURCES_EMPTY,
  SOURCES_HINT,
  SOURCE_INVALID,
  URL_INVALID,
} from "../../lib/exchangeText";
import type { ExchangeSettingsArgs } from "../../queries/exchange";
import type { ExchangeOverview } from "../../types/exchange";

export interface SettingsForm {
  /** One Nostr relay per line; empty = the network's defaults. */
  nostr: string;
  relay: string;
  watchtower: string;
  /** One source per line. */
  sources: string;
  /** A new `user:password`; empty keeps the stored one. */
  auth: string;
  /** Clear was pressed: send "" whatever the field holds. */
  clearAuth: boolean;
}

export interface SettingsCheck {
  nostr: string | null;
  relay: string | null;
  watchtower: string | null;
  sources: string | null;
  auth: string | null;
  args: ExchangeSettingsArgs | null;
}

const HTTP_URL = /^https?:\/\/\S+$/i;
const SOURCE_URL = /^(bitcoind\+)?https?:\/\/\S+$/i;
const NOSTR_URL = /^wss?:\/\/\S+$/i;

/**
 * Read the settings form: each non-empty line of the Nostr relays is a
 * ws(s) URL (none: the defaults); the relay and the watchtower are http(s) URLs or
 * empty (no relay / no watchtower), each non-empty line of the sources is an
 * Esplora URL or `bitcoind+http(s)://…`, at least one, and a new RPC login
 * is `user:password`. The backend checks them again before it stores them.
 */
export function checkSettings(form: SettingsForm): SettingsCheck {
  const relay = form.relay.trim();
  const watchtower = form.watchtower.trim();
  const sources = form.sources
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l !== "");
  const auth = form.auth.trim();
  const bad = sources.find((s) => !SOURCE_URL.test(s));
  const nostr = form.nostr
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l !== "");
  const badNostr = nostr.find((n) => !NOSTR_URL.test(n));
  const out: SettingsCheck = {
    nostr: badNostr !== undefined ? NOSTR_INVALID(badNostr) : null,
    relay: relay !== "" && !HTTP_URL.test(relay) ? URL_INVALID : null,
    watchtower: watchtower !== "" && !HTTP_URL.test(watchtower) ? URL_INVALID : null,
    sources: bad !== undefined ? SOURCE_INVALID(bad) : sources.length === 0 ? SOURCES_EMPTY : null,
    auth: !form.clearAuth && auth !== "" && !/^[^:]+:.+$/.test(auth) ? AUTH_INVALID : null,
    args: null,
  };
  if (out.nostr || out.relay || out.watchtower || out.sources || out.auth) return out;
  out.args = {
    relayUrl: relay,
    watchtowerUrl: watchtower,
    btcSources: sources,
    bitcoindAuth: form.clearAuth ? "" : auth === "" ? null : auth,
    nostrRelays: nostr,
  };
  return out;
}

function FieldError({ testId, text }: { testId: string; text: string | null }) {
  if (!text) return null;
  return (
    <p role="alert" className="mt-1 text-xs text-red-600" data-testid={testId}>
      {text}
    </p>
  );
}

export function ExchangeSettingsDialog({
  overview,
  refusal,
  busy,
  onClose,
  onSave,
}: {
  overview: ExchangeOverview;
  refusal: string | null;
  busy: boolean;
  onClose: () => void;
  onSave: (args: ExchangeSettingsArgs) => Promise<void>;
}) {
  const [form, setForm] = useState<SettingsForm>({
    nostr: overview.nostrRelays.join("\n"),
    relay: overview.relayUrl,
    watchtower: overview.watchtowerUrl ?? "",
    sources: overview.btcSources.join("\n"),
    auth: "",
    clearAuth: false,
  });
  const [error, setError] = useState<string | null>(null);
  const check = checkSettings(form);
  const set = (patch: Partial<SettingsForm>) => {
    setForm((f) => ({ ...f, ...patch }));
    setError(null);
  };
  const submit = async () => {
    if (!check.args) return;
    try {
      await onSave(check.args);
    } catch (e) {
      setError(mapError(e));
    }
  };

  return (
    <Dialog
      open
      onClose={() => !busy && onClose()}
      title="Exchange settings"
      className="max-w-[520px]"
    >
      <div className="space-y-3 text-sm">
        <div className="flex flex-col gap-1">
          <label htmlFor="exchange-nostr" className="text-sm font-medium text-gray-700">
            Nostr relays, one per line
          </label>
          <textarea
            id="exchange-nostr"
            data-testid="settings-nostr"
            rows={3}
            spellCheck={false}
            placeholder={overview.defaultNostrRelays.join("\n")}
            value={form.nostr}
            onChange={(e) => set({ nostr: e.target.value })}
            className="rounded border border-gray-300 px-3 py-1.5 font-mono text-xs focus:border-transparent focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
          <p className="text-xs text-gray-500 [text-wrap:pretty]" data-testid="settings-nostr-hint">
            {NOSTR_HINT}
          </p>
          <FieldError testId="settings-nostr-error" text={check.nostr} />
        </div>
        <div>
          <Input
            label="Your own relay (optional)"
            id="exchange-relay"
            data-testid="settings-relay"
            autoComplete="off"
            spellCheck={false}
            placeholder="Empty: the Nostr relays above"
            value={form.relay}
            onChange={(e) => set({ relay: e.target.value })}
          />
          <p
            className="mt-1 text-xs text-gray-500 [text-wrap:pretty]"
            data-testid="settings-relay-hint"
          >
            {RELAY_HINT}
          </p>
          <FieldError testId="settings-relay-error" text={check.relay} />
        </div>
        <div>
          <Input
            label="Watchtower URL (optional)"
            id="exchange-watchtower"
            data-testid="settings-watchtower"
            autoComplete="off"
            spellCheck={false}
            placeholder="Empty: no watchtower"
            value={form.watchtower}
            onChange={(e) => set({ watchtower: e.target.value })}
          />
          <FieldError testId="settings-watchtower-error" text={check.watchtower} />
        </div>
        <div className="flex flex-col gap-1">
          <label htmlFor="exchange-sources" className="text-sm font-medium text-gray-700">
            Bitcoin sources, one per line
          </label>
          <textarea
            id="exchange-sources"
            data-testid="settings-sources"
            rows={3}
            spellCheck={false}
            placeholder={"https://mempool.space/api\nbitcoind+http://127.0.0.1:18443"}
            value={form.sources}
            onChange={(e) => set({ sources: e.target.value })}
            className="rounded border border-gray-300 px-3 py-1.5 font-mono text-xs focus:border-transparent focus:outline-none focus:ring-2 focus:ring-blue-500"
          />
          <p
            className="text-xs text-gray-500 [text-wrap:pretty]"
            data-testid="settings-sources-hint"
          >
            {SOURCES_HINT} An Esplora base URL looks like https://mempool.space/api; a local Bitcoin
            Core is bitcoind+http://127.0.0.1:18443.
          </p>
          <FieldError testId="settings-sources-error" text={check.sources} />
        </div>
        <div>
          <div className="flex items-end gap-2">
            <div className="min-w-0 flex-1">
              <Input
                label="bitcoind RPC login (user:password)"
                id="exchange-auth"
                type="password"
                data-testid="settings-auth"
                autoComplete="off"
                disabled={form.clearAuth}
                placeholder={overview.bitcoindAuthSet && !form.clearAuth ? "set" : ""}
                value={form.clearAuth ? "" : form.auth}
                onChange={(e) => set({ auth: e.target.value })}
              />
            </div>
            {(overview.bitcoindAuthSet || form.clearAuth) && (
              <Button
                variant="secondary"
                data-testid="settings-auth-clear"
                onClick={() => set({ clearAuth: !form.clearAuth, auth: "" })}
              >
                {form.clearAuth ? "Keep" : "Clear"}
              </Button>
            )}
          </div>
          {overview.bitcoindAuthSet && (
            <p className="mt-1 text-xs text-gray-500" data-testid="settings-auth-state">
              {form.clearAuth ? AUTH_CLEARED : AUTH_SET}
            </p>
          )}
          <FieldError testId="settings-auth-error" text={check.auth} />
        </div>
        {error && (
          <p role="alert" className="text-xs text-red-600" data-testid="settings-error">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" data-testid="settings-cancel" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <GatedButton
            variant="primary"
            data-testid="settings-save"
            refusal={refusal}
            disabled={check.args === null}
            loading={busy}
            onClick={() => void submit()}
          >
            {busy ? SECURE_WINDOW_WAIT : "Save"}
          </GatedButton>
        </div>
      </div>
    </Dialog>
  );
}
