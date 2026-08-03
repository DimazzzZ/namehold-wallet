import { useState } from "react";
import { useSignNameMessage } from "../../queries/wallet";
import { Button } from "../ui/Button";
import { writeText } from "../../lib/clipboard";
import { displayName } from "../../lib/idn";
import { mapError } from "../../lib/errors";
import type { NameActionCapabilities, NameSignature } from "../../types";

/**
 * "Sign message" panel (Task 3): lets the wallet prove ownership of a name by
 * signing an exact piece of text with the owning key — reproduces hsrd's
 * `signmessagewithname` byte-for-byte, which is what Namebase's domain-claim
 * verification flow asks for (paste an exact message, sign it, paste the
 * signature back).
 *
 * Owner-only: renders nothing unless `caps.ownsName` — signing is meaningless
 * (and the backend would reject it) for a name this wallet doesn't hold.
 * The RAW name is sent to the backend; only the heading/placeholder render
 * through `displayName` for IDN labels.
 */
export function NameSignMessage({
  name,
  profileId,
  caps,
}: {
  name: string;
  profileId: string | null;
  caps: NameActionCapabilities | null | undefined;
}) {
  const decoded = displayName(name);
  const sign = useSignNameMessage();
  const [message, setMessage] = useState("");
  const [result, setResult] = useState<NameSignature | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showDetails, setShowDetails] = useState(false);
  const [copied, setCopied] = useState<"signature" | "publicKey" | "address" | null>(null);

  if (!caps?.ownsName) return null;

  const handleSign = async () => {
    setError(null);
    try {
      const res = await sign.run(name, message, profileId);
      setResult(res);
    } catch (e) {
      setError(mapError(e, "sign"));
    }
  };

  const copy = async (value: string, which: "signature" | "publicKey" | "address") => {
    await writeText(value);
    setCopied(which);
    setTimeout(() => setCopied(null), 2000);
  };

  return (
    <section className="space-y-2" data-testid="name-sign-message">
      <div className="font-medium text-gray-700">Sign message for .{decoded}</div>
      <p className="text-xs text-gray-500">
        Paste the exact text a third party (e.g. Namebase) gave you to verify
        ownership of this name, then Sign with your wallet key.
      </p>
      <textarea
        className="w-full border border-gray-300 rounded px-2 py-1 font-mono text-xs h-16"
        value={message}
        onChange={(e) => setMessage(e.target.value)}
        placeholder={`Namebase registry: I verify ownership of "${decoded}" for account #12345.`}
        data-testid="sign-message-input"
      />
      {error && (
        <div className="text-xs text-red-700" role="alert">
          {error}
        </div>
      )}
      <div>
        <Button
          size="sm"
          disabled={!message.trim() || sign.pending}
          onClick={handleSign}
          data-testid="sign-message-button"
        >
          {sign.pending ? "Signing…" : "Sign"}
        </Button>
      </div>

      {result && (
        <div className="space-y-2 border-t border-gray-200 pt-2">
          <div>
            <div className="text-xs text-gray-500">Signature</div>
            <div className="flex items-start gap-2">
              <code
                className="flex-1 break-all font-mono text-xs bg-gray-50 border border-gray-200 rounded p-1"
                data-testid="sign-message-signature"
              >
                {result.signature}
              </code>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => copy(result.signature, "signature")}
                data-testid="copy-signature"
              >
                {copied === "signature" ? "Copied!" : "Copy"}
              </Button>
            </div>
          </div>

          <button
            type="button"
            className="text-xs text-blue-600 hover:underline"
            onClick={() => setShowDetails((s) => !s)}
            data-testid="sign-message-details-toggle"
          >
            {showDetails ? "Hide" : "Show"} public key / address
          </button>

          {showDetails && (
            <div className="space-y-1 text-xs text-gray-600">
              <div>if Namebase asks for the public key</div>
              <div className="flex items-start gap-2">
                <code
                  className="flex-1 break-all font-mono bg-gray-50 border border-gray-200 rounded p-1"
                  data-testid="sign-message-pubkey"
                >
                  {result.publicKey}
                </code>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => copy(result.publicKey, "publicKey")}
                  data-testid="copy-pubkey"
                >
                  {copied === "publicKey" ? "Copied!" : "Copy"}
                </Button>
              </div>
              <div className="flex items-start gap-2">
                <code
                  className="flex-1 break-all font-mono bg-gray-50 border border-gray-200 rounded p-1"
                  data-testid="sign-message-address"
                >
                  {result.address}
                </code>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => copy(result.address, "address")}
                  data-testid="copy-address"
                >
                  {copied === "address" ? "Copied!" : "Copy"}
                </Button>
              </div>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
