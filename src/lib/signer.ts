import type { SignerSessionSummary } from "../types";

/** Whether the unlocked signer session belongs to `profileId`. The backend
 *  keeps one session for one wallet and refuses to sign for any other, so a
 *  session that is unlocked for another profile is, for this one, locked. */
export function isSignerUnlockedFor(
  signer: SignerSessionSummary | null | undefined,
  profileId: string | null | undefined,
): boolean {
  return !!signer?.unlocked && !!profileId && signer.walletProfileId === profileId;
}
