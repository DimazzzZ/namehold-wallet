import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const writeTextMock = vi.fn();
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: (...args: unknown[]) => writeTextMock(...args),
  readText: vi.fn().mockResolvedValue(""),
}));

import { NameSignMessage } from "../NameSignMessage";
import type { NameActionCapabilities } from "../../../types";

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

const cap = { allowed: false, reason: null };

function capsFor(name: string, ownsName: boolean): NameActionCapabilities {
  return {
    name,
    phase: "CLOSED",
    taskState: "ownedNoUrgentAction",
    ownsName,
    hasBidCommitment: false,
    hasBidCoin: false,
    hasRevealCoin: false,
    hasOwnerCoin: ownsName,
    revealTxid: null,
    bidValueDoos: null,
    canOpen: cap,
    canBid: cap,
    canReveal: cap,
    canRedeem: cap,
    canRegister: cap,
    canUpdate: cap,
    canTransfer: cap,
    canFinalize: cap,
    canCancelTransfer: cap,
    canRenew: cap,
    canRevoke: cap,
    nextActionKey: null,
    nextActionLabel: null,
    nextActionReason: null,
    countdownLabel: null,
    countdownBlocks: null,
    countdownHours: null,
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  writeTextMock.mockReset();
});

describe("NameSignMessage — sign an arbitrary message with the owning key (Task 3)", () => {
  it("does not render for a name the wallet does not own", () => {
    render(
      <NameSignMessage name="notmine" profileId="p1" caps={capsFor("notmine", false)} />,
      { wrapper: wrapper() },
    );
    expect(screen.queryByTestId("name-sign-message")).not.toBeInTheDocument();
  });

  it("does not render when caps have not loaded yet", () => {
    render(<NameSignMessage name="ecology" profileId="p1" caps={undefined} />, {
      wrapper: wrapper(),
    });
    expect(screen.queryByTestId("name-sign-message")).not.toBeInTheDocument();
  });

  it("signs a typed message and shows the returned signature; Copy puts it on the clipboard", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "sign_name_message") {
        return Promise.resolve({
          signature: "c2lnbmF0dXJlLWJhc2U2NA==",
          publicKey: "02aabbccddeeff",
          address: "hs1qownercoinaddress",
        });
      }
      return Promise.resolve(null);
    });

    render(
      <NameSignMessage name="ecology" profileId="p1" caps={capsFor("ecology", true)} />,
      { wrapper: wrapper() },
    );

    fireEvent.change(screen.getByTestId("sign-message-input"), {
      target: {
        value: 'Namebase registry: I verify ownership of "ecology" for account #20544.',
      },
    });
    fireEvent.click(screen.getByTestId("sign-message-button"));

    expect(await screen.findByTestId("sign-message-signature")).toHaveTextContent(
      "c2lnbmF0dXJlLWJhc2U2NA==",
    );

    // sign_name_message must have been called with the RAW name + exact typed message.
    const call = invokeMock.mock.calls.find((c) => c[0] === "sign_name_message");
    expect(call?.[1]).toMatchObject({
      name: "ecology",
      message: 'Namebase registry: I verify ownership of "ecology" for account #20544.',
      walletProfileId: "p1",
    });

    fireEvent.click(screen.getByTestId("copy-signature"));
    await waitFor(() => expect(writeTextMock).toHaveBeenCalledWith("c2lnbmF0dXJlLWJhc2U2NA=="));

    // publicKey/address are behind a collapsible, captioned for Namebase's pubkey ask.
    expect(screen.queryByTestId("sign-message-pubkey")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("sign-message-details-toggle"));
    expect(screen.getByTestId("sign-message-pubkey")).toHaveTextContent("02aabbccddeeff");
    expect(screen.getByTestId("sign-message-address")).toHaveTextContent("hs1qownercoinaddress");
    expect(screen.getByText(/if Namebase asks for the public key/i)).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("copy-pubkey"));
    await waitFor(() => expect(writeTextMock).toHaveBeenCalledWith("02aabbccddeeff"));
  });

  it("a locked signer unlocks first, then retries sign — both invokes fire in order", async () => {
    let signCalls = 0;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "sign_name_message") {
        signCalls += 1;
        if (signCalls === 1) {
          return Promise.reject(new Error("Wallet locked"));
        }
        return Promise.resolve({
          signature: "c2Vjb25kLXRyeQ==",
          publicKey: "03112233",
          address: "hs1qaddr",
        });
      }
      if (cmd === "unlock_local_signer") {
        return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: 0 });
      }
      return Promise.resolve(null);
    });

    render(
      <NameSignMessage name="ecology" profileId="p1" caps={capsFor("ecology", true)} />,
      { wrapper: wrapper() },
    );

    fireEvent.change(screen.getByTestId("sign-message-input"), {
      target: { value: "verify me" },
    });
    fireEvent.click(screen.getByTestId("sign-message-button"));

    expect(await screen.findByTestId("sign-message-signature")).toHaveTextContent(
      "c2Vjb25kLXRyeQ==",
    );

    const cmdOrder = invokeMock.mock.calls.map((c) => c[0]);
    expect(cmdOrder).toEqual(["sign_name_message", "unlock_local_signer", "sign_name_message"]);
  });

  it("punycode: sends the RAW xn-- name to the backend, but renders the decoded Unicode heading", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "sign_name_message") {
        return Promise.resolve({ signature: "c2ln", publicKey: "02aa", address: "hs1qaddr" });
      }
      return Promise.resolve(null);
    });

    render(
      <NameSignMessage
        name="xn--e1adigm"
        profileId="p1"
        caps={capsFor("xn--e1adigm", true)}
      />,
      { wrapper: wrapper() },
    );

    // `xn--e1adigm` decodes to "козел" — the heading renders the pretty form.
    expect(screen.getByText(/sign message for \.козел/i)).toBeInTheDocument();
    expect(screen.queryByText(/sign message for \.xn--e1adigm/i)).not.toBeInTheDocument();

    fireEvent.change(screen.getByTestId("sign-message-input"), {
      target: { value: "verify" },
    });
    fireEvent.click(screen.getByTestId("sign-message-button"));

    await screen.findByTestId("sign-message-signature");
    const call = invokeMock.mock.calls.find((c) => c[0] === "sign_name_message");
    expect(call?.[1]).toMatchObject({ name: "xn--e1adigm" });
  });
});
