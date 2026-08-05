import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { dollarydoosToHns } from "../../lib/utils";

interface PaidSwapOffer {
  name: string;
  buyerAddress: string;
  priceDoos: number;
  transferTxid: string | null;
  claimed: boolean;
  createdAt: string;
}

interface ClaimResult {
  verified: boolean;
  paidDoos: number;
  confirmations: number;
}

/**
 * Paid swap claim section: shown in NameActionsModal when a paid_swap_offer
 * exists for the current name. Three states:
 * 1. Awaiting buyer — transfer done, waiting for buyer's finalize-with-payment.
 * 2. Ready to claim — buyer's tx is on-chain, seller can verify + mark claimed.
 * 3. Claimed — payment verified.
 */
export function PaidSwapClaim({ name }: { name: string }) {
  const queryClient = useQueryClient();
  const [claimTxid, setClaimTxid] = useState("");

  const { data: offer, isLoading } = useQuery<PaidSwapOffer | null>({
    queryKey: ["paid-swap-offer", name],
    queryFn: async () => {
      const result = await invoke<PaidSwapOffer | null>("get_paid_swap_offer", { name });
      return result;
    },
    staleTime: 10_000,
  });

  const claimMutation = useMutation<ClaimResult, Error, { txid: string }>({
    mutationFn: async ({ txid }) => {
      return await invoke<ClaimResult>("claim_paid_transfer", { name, txid });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["paid-swap-offer", name] });
    },
  });

  const removeMutation = useMutation<void, Error>({
    mutationFn: async () => {
      await invoke("remove_paid_swap_offer", { name });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["paid-swap-offer", name] });
    },
  });

  if (isLoading || !offer) return null;

  // State 3: Claimed
  if (offer.claimed) {
    return (
      <div className="rounded border border-green-200 bg-green-50 p-3 space-y-1">
        <div className="font-medium text-green-800">✓ Payment claimed</div>
        <div className="text-sm text-green-700">
          Received {dollarydoosToHns(offer.priceDoos)} HNS from buyer.
        </div>
        {offer.transferTxid && (
          <div className="text-xs text-green-600 font-mono truncate">
            TX: {offer.transferTxid}
          </div>
        )}
      </div>
    );
  }

  // State 1 & 2: Awaiting / Ready to claim
  return (
    <div className="rounded border border-amber-200 bg-amber-50 p-3 space-y-2">
      <div className="font-medium text-amber-800">
        Paid swap offer — {dollarydoosToHns(offer.priceDoos)} HNS
      </div>
      <div className="text-sm text-amber-700">
        Buyer: <span className="font-mono text-xs">{offer.buyerAddress}</span>
      </div>
      <div className="text-sm text-amber-700">
        Status: Awaiting buyer&apos;s finalize-with-payment transaction.
      </div>

      <div className="space-y-2 pt-2 border-t border-amber-200">
        <Input
          label="Buyer's finalize-with-payment TX ID"
          value={claimTxid}
          onChange={(e) => setClaimTxid(e.target.value)}
          placeholder="Enter txid…"
        />
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="primary"
            disabled={!claimTxid.trim() || claimMutation.isPending}
            onClick={() => claimMutation.mutate({ txid: claimTxid.trim() })}
          >
            {claimMutation.isPending ? "Verifying…" : "Claim payment"}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            disabled={removeMutation.isPending}
            onClick={() => removeMutation.mutate()}
          >
            Cancel offer
          </Button>
        </div>
        {claimMutation.isError && (
          <div className="text-sm text-red-600">
            {claimMutation.error?.message || "Verification failed"}
          </div>
        )}
        {claimMutation.isSuccess && !claimMutation.data?.verified && (
          <div className="text-sm text-red-600">
            Payment not found in this transaction. Check the TX ID.
          </div>
        )}
      </div>
    </div>
  );
}
