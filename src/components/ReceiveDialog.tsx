import { useEffect, useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import type { WalletProfileSummary } from "../types";
import { useReceiveAddresses, useRevealNextReceiveAddress } from "../queries/read";
import { explorerAddressUrl, explorerCoversNetwork, openExternal } from "../lib/openExternal";
import { mapError } from "../lib/errors";
import { truncateMiddle } from "../lib/utils";
import { useUiStore } from "../stores/ui";
import { ReceiveAddressList } from "./ReceiveAddressList";
import { NetworkBadge } from "./ui/NetworkBadge";
import { Icon } from "./ui/Icon";
import { ArrowUpRight, Plus } from "lucide-react";
import { Button } from "./ui/Button";
import { CopyField } from "./ui/CopyField";
import { Dialog } from "./ui/Dialog";
import { Disclosure } from "./ui/Disclosure";

interface ReceiveDialogProps {
  open: boolean;
  onClose: () => void;
  profile: WalletProfileSummary;
}

/** Everything about receiving HNS: the address + QR, every derived address, the xpub. */
export function ReceiveDialog({ open, onClose, profile }: ReceiveDialogProps) {
  const showToast = useUiStore((s) => s.showToast);
  const revealNext = useRevealNextReceiveAddress();
  const { data: addresses } = useReceiveAddresses();
  // `reveal_next_receive_address` deliberately leaves profile.receiveAddress
  // as it is (see queries/read.ts), so the new address is shown from here —
  // otherwise "New address" would look like it did nothing.
  const [fresh, setFresh] = useState<string | null>(null);
  useEffect(() => setFresh(null), [open, profile.id]);

  const address = fresh ?? profile.receiveAddress;
  const mainnet = profile.network === "mainnet";

  const handleNewAddress = async () => {
    try {
      setFresh(await revealNext.mutateAsync({ walletProfileId: profile.id }));
      showToast("New address generated", "success");
    } catch (e) {
      showToast(mapError(e, "build"), "error");
    }
  };

  if (!open) return null;

  return (
    <Dialog open onClose={onClose} title="Receive HNS" className="max-w-xl">
      <div className="space-y-3" data-testid="receive-dialog">
        {address ? (
          <div className="flex gap-4 items-center">
            <QRCodeSVG value={address} size={120} level="M" className="shrink-0" />
            <div className="min-w-0 flex-1">
              <CopyField
                layout="stacked"
                label={
                  <span className="flex flex-wrap items-center gap-2 text-xs">
                    <span>Your address</span>
                    <NetworkBadge network={profile.network} />
                    {!mainnet && (
                      <span className="text-amber-600">
                        — {profile.network} addresses differ from mainnet
                      </span>
                    )}
                  </span>
                }
                value={address}
                valueTestId="receive-address-value"
                copyLabel="Copy address"
                toastLabel="Address"
                extraActions={
                  <>
                    {explorerCoversNetwork(profile.network) && (
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => openExternal(explorerAddressUrl(address))}
                        data-testid="receive-address-explorer-link"
                        className="inline-flex items-center gap-1"
                      >
                        <Icon icon={ArrowUpRight} size={13} /> Explorer
                      </Button>
                    )}
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => void handleNewAddress()}
                      disabled={revealNext.isPending}
                      data-testid="receive-new-address"
                      className="inline-flex items-center gap-1"
                    >
                      <Icon icon={Plus} size={13} />
                      {revealNext.isPending ? "Generating…" : "New address"}
                    </Button>
                  </>
                }
              />
            </div>
          </div>
        ) : (
          <div className="text-sm text-gray-500">No address derived yet. Try syncing.</div>
        )}

        <div className="border-t border-gray-100 pt-3">
          <Disclosure
            summary={`All addresses${addresses ? ` (${addresses.length})` : ""}`}
            testId="receive-all-addresses"
          >
            <ReceiveAddressList />
          </Disclosure>
        </div>

        {/* If a multisig wallet kind is ever added, gate this on single-sig:
            xpub-derived payee setups only support single-signature xpubs. */}
        <Disclosure
          summary={
            <span className="flex items-center gap-2">
              <span>Advanced: account public key (xpub)</span>
              <span className="font-mono text-xs text-gray-400">
                {truncateMiddle(profile.accountXpub)}
              </span>
            </span>
          }
        >
          <CopyField
            value={profile.accountXpub}
            valueTestId="account-xpub-value"
            copyTestId="copy-xpub"
            copyLabel="Copy public key"
            toastLabel="Account public key"
          />
          <p className="text-xs text-gray-500 mt-2 [text-wrap:pretty]">
            For payees that derive addresses from an xpub (e.g. an exported Namebase setup). This is
            a single-signature wallet, so the addresses derived from it are yours to spend. Anyone
            with this key can see every address and balance you'll ever use, but cannot move your
            funds.
          </p>
        </Disclosure>
      </div>
    </Dialog>
  );
}
