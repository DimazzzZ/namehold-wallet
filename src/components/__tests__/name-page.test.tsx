import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// The name page itself (`/names/:name`): routing and IDN, the header primary,
// the ⋯ menu, type-to-confirm Revoke, the write gate and network timing.
// Draft cleanup on leave is pinned in name-actions-draft-cleanup.test.tsx.

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { NamePageAt } from "../../test/namePage";
import { allowed, makeCapabilities, refused } from "../../test/fixtures/capabilities";
import { makeProfile, makeSession } from "../../test/fixtures/wallet";
import { loadSettings } from "../../test/fixtures/settings";
import { useUiStore } from "../../stores/ui";
import type { NameActionCapabilities, WalletProfileSummary } from "../../types";

interface RouteOpts {
  name?: string;
  state?: string;
  caps?: Partial<NameActionCapabilities> | null;
  info?: Record<string, unknown>;
  profile?: Partial<WalletProfileSummary>;
  canWrite?: boolean;
  writeReason?: string | null;
}

function route(o: RouteOpts = {}) {
  const name = o.name ?? "ownedname";
  return (cmd: string) => {
    switch (cmd) {
      case "get_name_action_capabilities":
        return Promise.resolve(o.caps === null ? null : makeCapabilities({ name, ...o.caps }));
      case "list_wallet_profiles":
        return Promise.resolve([makeProfile({ network: "regtest", ...o.profile })]);
      case "get_signer_session":
        return Promise.resolve(makeSession({ unlocked: o.canWrite ?? true }));
      case "get_write_capability": {
        const canWrite = o.canWrite ?? true;
        return Promise.resolve({
          signerUnlocked: canWrite,
          broadcasterAvailable: true,
          canWrite,
          reason: canWrite ? null : (o.writeReason ?? null),
        });
      }
      case "read_name_info":
        return Promise.resolve({
          name,
          state: o.state ?? "CLOSED",
          height: 100,
          renewal: 200,
          owner: { hash: "ab".repeat(32), index: 0 },
          registered: true,
          value: 1_000_000,
          highest: 2_000_000,
          stats: { blocksUntilExpire: 98_120 },
          ...o.info,
        });
      case "build_revoke_draft":
      case "build_renew_draft":
      case "build_open_draft":
        return Promise.resolve({ id: "draft-1" });
      case "sign_tx_draft":
        return Promise.resolve({ id: "draft-1" });
      case "broadcast_tx_draft":
        return Promise.resolve({ txid: "abcdef0123456789" });
      default:
        return Promise.resolve(null);
    }
  };
}

const owned: Partial<NameActionCapabilities> = {
  phase: "CLOSED",
  taskState: "ownedNoUrgentAction",
  ownsName: true,
  nameIsRegistered: true,
  hasOwnerCoin: true,
  canUpdate: allowed,
  canTransfer: allowed,
  canRenew: allowed,
  canRevoke: allowed,
};

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

const renderAt = (path: string) => render(<NamePageAt path={path} />, { wrapper: wrapper() });
const location = () => screen.getByTestId("location");
/** Every filled blue button on screen. */
const blueButtons = () =>
  screen.getAllByRole("button").filter((b) => b.className.split(" ").includes("bg-blue-600"));
const nameArgs = () =>
  invokeMock.mock.calls
    .map((c) => (c[1] as { name?: unknown } | undefined)?.name)
    .filter((n) => n !== undefined);

beforeEach(() => {
  invokeMock.mockReset();
  loadSettings();
  useUiStore.setState({ toastQueue: [], toastMessage: null });
});

describe("Name page — routing", () => {
  it.each(["/names/MÜNCHEN?tab=dns", "/names/münchen?tab=dns"])(
    "redirects %s to the raw ACE form, and only ACE reaches the backend",
    async (path) => {
      invokeMock.mockImplementation(route({ name: "xn--mnchen-3ya", caps: owned }));
      renderAt(path);

      await waitFor(() => expect(location()).toHaveTextContent("/names/xn--mnchen-3ya?tab=dns"));
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith("read_name_info", { name: "xn--mnchen-3ya" }),
      );
      expect(new Set(nameArgs())).toEqual(new Set(["xn--mnchen-3ya"]));
      expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
        ".münchen.xn--mnchen-3ya",
      );
    },
  );

  it("shows an invalid name and calls no backend", async () => {
    invokeMock.mockImplementation(route());
    renderAt("/names/xn--zz");

    expect(screen.getByTestId("name-invalid")).toHaveTextContent(
      "This isn't a valid Handshake name.",
    );
    expect(screen.getByRole("link", { name: "Names" })).toHaveAttribute("href", "/names");
    await new Promise((r) => setTimeout(r, 0));
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("falls back to Overview on an unknown ?tab=, and tab changes rewrite the URL", async () => {
    invokeMock.mockImplementation(route({ caps: owned }));
    renderAt("/names/ownedname?tab=nope");

    expect(await screen.findByRole("tab", { name: /^Overview/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    fireEvent.click(screen.getByRole("tab", { name: /^DNS records/ }));
    expect(location()).toHaveTextContent("/names/ownedname?tab=dns");
    fireEvent.click(screen.getByRole("tab", { name: /^Overview/ }));
    expect(location()).toHaveTextContent(/^\/names\/ownedname$/);
  });
});

describe("Name page — header primary", () => {
  const cases: [string, string, Partial<NameActionCapabilities>, string][] = [
    ["availableToOpen", "AVAILABLE", { canOpen: allowed }, "Open auction"],
    ["readyToReveal", "REVEAL", { canReveal: allowed, hasBidCommitment: true }, "Reveal"],
    ["wonNeedsRegister", "CLOSED", { canRegister: allowed, ownsName: true }, "Register"],
    ["lostNeedsRedeem", "CLOSED", { canRedeem: allowed }, "Redeem"],
    [
      "transferPendingFinalize",
      "CLOSED",
      { ...owned, transferPending: true, canFinalize: allowed },
      "Finalize transfer",
    ],
    ["ownedNoUrgentAction", "CLOSED", owned, "Renew"],
    ["expiringSoon", "CLOSED", owned, "Renew"],
  ];

  it.each(cases)("%s → %s", async (taskState, state, caps, label) => {
    invokeMock.mockImplementation(
      route({
        state,
        caps: { phase: state, ...caps, taskState: taskState as never },
      }),
    );
    renderAt("/names/ownedname");

    await waitFor(() => expect(screen.getByTestId("name-primary")).toHaveTextContent(label));
    expect(screen.getByTestId("name-primary")).toBeEnabled();
    // The header primary is the page's one blue button.
    expect(blueButtons()).toEqual([screen.getByTestId("name-primary")]);
  });

  it("has no header primary in BIDDING: the bid form's Bid is the one blue button", async () => {
    invokeMock.mockImplementation(
      route({
        state: "BIDDING",
        info: { stats: { blocksUntilReveal: 10 } },
        caps: { phase: "BIDDING", taskState: "readyToBid", canBid: allowed },
      }),
    );
    renderAt("/names/ownedname");

    await screen.findByLabelText(/^Bid \(HNS\)/);
    expect(screen.queryByTestId("name-primary")).not.toBeInTheDocument();
    expect(blueButtons().map((b) => b.textContent)).toEqual(["Bid"]);
  });

  it("Reveal opens the confirm panel and hides itself: Confirm & reveal is the one blue button", async () => {
    invokeMock.mockImplementation(
      route({
        state: "REVEAL",
        info: { stats: { blocksUntilClose: 10 } },
        caps: {
          phase: "REVEAL",
          taskState: "readyToReveal",
          canReveal: allowed,
          hasBidCommitment: true,
        },
      }),
    );
    renderAt("/names/ownedname?tab=dns");

    fireEvent.click(await screen.findByTestId("name-primary"));
    expect(await screen.findByTestId("reveal-confirm-panel")).toBeInTheDocument();
    expect(location()).toHaveTextContent(/^\/names\/ownedname$/);
    expect(screen.queryByTestId("name-primary")).not.toBeInTheDocument();
    expect(blueButtons()).toEqual([screen.getByTestId("reveal-confirm-broadcast")]);
  });

  it("is disabled with 'Recover your bid commitment first' when the commitment is missing", async () => {
    invokeMock.mockImplementation(
      route({
        state: "REVEAL",
        caps: {
          phase: "REVEAL",
          taskState: "readyToReveal",
          canReveal: allowed,
          hasBidCommitment: false,
        },
      }),
    );
    renderAt("/names/ownedname");

    const primary = await screen.findByTestId("name-primary");
    await waitFor(() => expect(primary).toBeDisabled());
    fireEvent.mouseEnter(primary.parentElement!);
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      "Recover your bid commitment first (below).",
    );
  });

  it("is disabled with the capability's own reason", async () => {
    invokeMock.mockImplementation(
      route({ caps: { ...owned, canRenew: refused("owner coin not synced") } }),
    );
    renderAt("/names/ownedname");

    const primary = await screen.findByTestId("name-primary");
    await waitFor(() => expect(primary).toBeDisabled());
    fireEvent.mouseEnter(primary.parentElement!);
    expect(await screen.findByRole("tooltip")).toHaveTextContent("owner coin not synced");
  });

  it("is hidden while an action of ours waits for a block", async () => {
    invokeMock.mockImplementation(route({ caps: { ...owned, pendingBroadcastAction: "renew" } }));
    renderAt("/names/ownedname");

    await waitFor(() =>
      expect(screen.getByTestId("name-phase")).toHaveTextContent(
        "Renew sent · waiting for a block",
      ),
    );
    expect(screen.queryByTestId("name-primary")).not.toBeInTheDocument();
  });

  it("stays on the page after a broadcast and toasts it", async () => {
    invokeMock.mockImplementation(route({ caps: owned }));
    renderAt("/names/ownedname");

    const primary = await screen.findByTestId("name-primary");
    await waitFor(() => expect(primary).toBeEnabled());
    fireEvent.click(primary);
    await waitFor(() =>
      expect(useUiStore.getState().toastQueue.slice(-1)[0]?.message).toMatch(
        /^RENEW broadcast — abcdef012345…/,
      ),
    );
    expect(location()).toHaveTextContent(/^\/names\/ownedname$/);
  });
});

describe("Name page — write gate", () => {
  it("watch-only: the gate without Unlock, and the primary disabled with the reason", async () => {
    const reason = "This is a watch-only wallet: it can't sign.";
    invokeMock.mockImplementation(
      route({
        caps: owned,
        profile: { watchOnly: true, kind: "watch_only_xpub" },
        canWrite: false,
        writeReason: reason,
      }),
    );
    renderAt("/names/ownedname");

    await waitFor(() =>
      expect(screen.getByTestId("name-actions-blocked")).toHaveTextContent(reason),
    );
    expect(screen.queryByTestId("unlock-now")).not.toBeInTheDocument();
    const primary = screen.getByTestId("name-primary");
    expect(primary).toBeDisabled();
    fireEvent.mouseEnter(primary.parentElement!);
    expect(await screen.findByRole("tooltip")).toHaveTextContent(reason);
  });

  it("locked hot wallet: the gate carries Unlock", async () => {
    invokeMock.mockImplementation(
      route({ caps: owned, canWrite: false, writeReason: "Unlock your wallet to sign." }),
    );
    renderAt("/names/ownedname");

    await screen.findByTestId("name-actions-blocked");
    await waitFor(() => expect(screen.getByTestId("unlock-now")).toBeInTheDocument());
    expect(screen.getByTestId("name-primary")).toBeDisabled();
  });
});

describe("Name page — ⋯ menu", () => {
  const openMenu = async () => fireEvent.click(await screen.findByTestId("name-more"));
  const item = (name: string) => screen.getByRole("menuitem", { name });

  it.each([
    ["Transfer to another wallet…", "/names/ownedname?tab=transfer"],
    ["DNS records", "/names/ownedname?tab=dns"],
    ["Sign a message…", "/names/ownedname?tab=sign"],
  ])("%s goes to its tab", async (label, path) => {
    invokeMock.mockImplementation(route({ caps: owned }));
    renderAt("/names/ownedname");
    await openMenu();
    fireEvent.click(item(label));
    expect(location()).toHaveTextContent(path);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("Revoke… opens the confirm box when Revoke is allowed", async () => {
    invokeMock.mockImplementation(route({ caps: owned }));
    renderAt("/names/ownedname");
    await waitFor(() => expect(screen.getByTestId("name-primary")).toBeEnabled());
    await openMenu();
    fireEvent.click(item("Revoke…"));
    expect(location()).toHaveTextContent("/names/ownedname?tab=transfer");
    expect(await screen.findByTestId("revoke-confirm-input")).toBeInTheDocument();
  });

  it("Revoke… only goes to the tab when Revoke is refused", async () => {
    invokeMock.mockImplementation(route({ caps: { ...owned, canRevoke: refused("no") } }));
    renderAt("/names/ownedname");
    await waitFor(() => expect(screen.getByTestId("name-primary")).toBeEnabled());
    await openMenu();
    fireEvent.click(item("Revoke…"));
    expect(location()).toHaveTextContent("/names/ownedname?tab=transfer");
    await screen.findByTestId("revoke-open");
    expect(screen.queryByTestId("revoke-confirm-input")).not.toBeInTheDocument();
  });

  it("Sell on the market… opens the Lock dialog when selling is possible", async () => {
    invokeMock.mockImplementation(route({ caps: owned }));
    renderAt("/names/ownedname");
    await waitFor(() => expect(screen.getByTestId("name-primary")).toBeEnabled());
    await openMenu();
    fireEvent.click(item("Sell on the market…"));
    expect(await screen.findByText("Sell .ownedname")).toBeInTheDocument();
  });

  it("Sell on the market… goes to the tab when selling is refused", async () => {
    invokeMock.mockImplementation(route({ caps: { ...owned, canTransfer: refused("locked") } }));
    renderAt("/names/ownedname");
    await waitFor(() => expect(screen.getByTestId("name-primary")).toBeEnabled());
    await openMenu();
    fireEvent.click(item("Sell on the market…"));
    expect(location()).toHaveTextContent("/names/ownedname?tab=transfer");
    expect(screen.queryByText("Sell .ownedname")).not.toBeInTheDocument();
  });
});

describe("Name page — Revoke needs the name typed", () => {
  async function openRevoke() {
    invokeMock.mockImplementation(route({ caps: owned }));
    renderAt("/names/ownedname?tab=transfer");
    const open = await screen.findByTestId("revoke-open");
    await waitFor(() => expect(open).toBeEnabled());
    // A secondary Button in red text, so focus, disabled and busy match the rest.
    expect(open).toHaveClass("border-gray-300", "text-red-700", "hover:bg-red-50");
    fireEvent.click(open);
    return screen.getByTestId("revoke-confirm-input");
  }
  const revokeBuilds = () =>
    invokeMock.mock.calls.filter((c) => c[0] === "build_revoke_draft").length;

  it("stays disabled until the exact name is typed, then revokes once and stays", async () => {
    const input = await openRevoke();
    const confirm = screen.getByTestId("revoke-confirm");
    expect(confirm).toHaveTextContent("Revoke .ownedname");
    expect(confirm).toBeDisabled();
    fireEvent.change(input, { target: { value: "ownedname2" } });
    expect(confirm).toBeDisabled();
    fireEvent.change(input, { target: { value: " .ownedname " } });
    expect(confirm).toBeEnabled();
    expect(revokeBuilds()).toBe(0);

    fireEvent.click(confirm);
    await waitFor(() =>
      expect(useUiStore.getState().toastQueue.slice(-1)[0]?.message).toMatch(
        /^REVOKE broadcast — /,
      ),
    );
    expect(revokeBuilds()).toBe(1);
    // Success collapses the box; the page stays.
    await waitFor(() =>
      expect(screen.queryByTestId("revoke-confirm-input")).not.toBeInTheDocument(),
    );
    expect(location()).toHaveTextContent("/names/ownedname?tab=transfer");
  });

  it("Esc and Cancel close the box and clear the text", async () => {
    let input = await openRevoke();
    fireEvent.change(input, { target: { value: "ownedname" } });
    fireEvent.keyDown(input, { key: "Escape" });
    expect(screen.queryByTestId("revoke-confirm-input")).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId("revoke-open"));
    input = screen.getByTestId("revoke-confirm-input");
    expect(input).toHaveValue("");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByTestId("revoke-confirm-input")).not.toBeInTheDocument();
    expect(revokeBuilds()).toBe(0);
  });

  it("accepts the decoded form of an IDN", async () => {
    invokeMock.mockImplementation(route({ name: "xn--mnchen-3ya", caps: owned }));
    renderAt("/names/xn--mnchen-3ya?tab=transfer");
    const open = await screen.findByTestId("revoke-open");
    await waitFor(() => expect(open).toBeEnabled());
    fireEvent.click(open);
    fireEvent.change(screen.getByTestId("revoke-confirm-input"), {
      target: { value: "münchen" },
    });
    expect(screen.getByTestId("revoke-confirm")).toBeEnabled();
  });
});

describe("Name page — network timing", () => {
  it("regtest reads blocks only: no date anywhere", async () => {
    invokeMock.mockImplementation(route({ caps: owned, profile: { network: "regtest" } }));
    renderAt("/names/ownedname");

    await waitFor(() =>
      expect(screen.getByTestId("name-phase")).toHaveTextContent(
        "Owned · renew within 98,120 blocks",
      ),
    );
    expect(screen.getByTestId("name-details")).toHaveTextContent("in 98,120 blocks");
    const monthYear = /\b(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{4}\b/;
    expect(document.body.textContent).not.toMatch(monthYear);
    fireEvent.click(screen.getByRole("tab", { name: /^Transfer & sell/ }));
    expect(screen.getByTestId("renew-text")).toHaveTextContent(
      "It currently expires in 98,120 blocks.",
    );
    expect(document.body.textContent).not.toMatch(monthYear);
  });

  it("mainnet reads an approximate date, with the blocks as a note", async () => {
    invokeMock.mockImplementation(route({ caps: owned, profile: { network: "mainnet" } }));
    renderAt("/names/ownedname");

    await waitFor(() =>
      expect(screen.getByTestId("name-phase")).toHaveTextContent(
        /^Owned · renew by [A-Z][a-z]{2} \d{4} \(98,120 blocks\)$/,
      ),
    );
  });
});
