// Which sections of the name-actions modal exist right now, and which are
// still ahead.
//
// The modal used to render a flat catalogue of everything the wallet can do,
// filtered by one flag (`ownsName`). That flag is true during REVEAL for a
// wallet leading its own auction, so a name that had not been won showed DNS
// records, Ownership and Sign message — and auto-expanded them. The reverse
// failure is just as bad: hiding a section entirely reads as a missing
// feature rather than a later step.
//
// So a section is one of three things, and this module is the only place that
// decides which. It is pure, so the whole stage matrix is testable without
// rendering a 1000-line modal.

import type { NameActionCapabilities } from "../types";

export type SectionState =
  /** At least one action inside is allowed. Rendered open, with its controls. */
  | { kind: "live" }
  /** A later stage. One muted line naming what unlocks it — no controls. */
  | { kind: "upcoming"; when: string }
  /** Cannot apply to this name for this wallet. Not rendered at all. */
  | { kind: "absent" };

export interface ModalSections {
  /** Manual Open / Reveal / Redeem — the fallback for when the guided panel
   *  has fallen out of step with the chain. */
  auction: SectionState;
  /** DNS records: REGISTER and UPDATE. */
  records: SectionState;
  /** Transfer / Finalize / Cancel / Renew / Revoke, and signing for the name. */
  ownership: SectionState;
  /** Whether anything at all is actionable — drives the advanced toggle. */
  anyLive: boolean;
}

const NOT_REGISTERED = "after you register this name";

export function resolveSections(caps: NameActionCapabilities | null | undefined): ModalSections {
  // Between broadcast and the block the chain still reports the previous
  // state, so every phase-derived section would describe a world where the
  // user never pressed the button. The honest answer is that there is nothing
  // to do, and a menu of alternatives here only invites a competing
  // transaction. The guided panel says what is in flight.
  if (caps?.pendingBroadcastAction) {
    return {
      auction: { kind: "absent" },
      records: { kind: "absent" },
      ownership: { kind: "absent" },
      anyLive: false,
    };
  }

  const registered = caps?.nameIsRegistered === true;
  const ours = caps?.ownsName === true;

  const auctionLive =
    caps?.canOpen.allowed === true ||
    caps?.canReveal.allowed === true ||
    caps?.canRedeem.allowed === true;

  // Once the name is registered the auction is history — a standing fallback
  // for Open / Reveal / Redeem is noise on every name the wallet manages.
  // A redeem left over keeps it live, because that IS an auction step.
  const auction: SectionState = auctionLive
    ? { kind: "live" }
    : registered
      ? { kind: "absent" }
      : { kind: "upcoming", when: "when this name's auction needs a step from you" };

  // Register publishes the first resource, so it belongs to this section —
  // which means the just-won stage needs it open before the name is
  // registered. A pending transfer takes it away again: hsd accepts
  // TRANSFER -> UPDATE and that transition is the cancel, so Update here would
  // end the transfer while saying nothing about transfers.
  // The backend's own flag, the same `transfer_has_items` `can_update` keys
  // on. The task state is a different question — it comes from the phase
  // string — and where the two disagree this section would stand open over a
  // button the node refuses.
  const transferPending = caps?.transferPending === true;
  const records: SectionState = !ours
    ? { kind: "absent" }
    : transferPending
      ? {
          kind: "upcoming",
          when: "after the transfer settles — editing records now would cancel it, which Cancel transfer does on purpose",
        }
      : registered || caps?.canRegister.allowed === true
        ? { kind: "live" }
        : { kind: "upcoming", when: NOT_REGISTERED };

  const ownership: SectionState = !ours
    ? { kind: "absent" }
    : registered
      ? { kind: "live" }
      : { kind: "upcoming", when: NOT_REGISTERED };

  return {
    auction,
    records,
    ownership,
    anyLive: [auction, records, ownership].some((s) => s.kind === "live"),
  };
}
