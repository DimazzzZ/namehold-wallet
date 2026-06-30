import { describe, it, expect } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { SourceBadge } from "../ui/SourceBadge";
import type {
  ConnectionMode,
  ProviderStatus,
  ReadContext,
  ReadProviderKind,
} from "../../types";

function makeProvider(
  overrides: Partial<ProviderStatus> = {},
): ProviderStatus {
  return {
    kind: "local_hsd" as ReadProviderKind,
    label: "Local hsd",
    healthy: true,
    writeCapable: true,
    manageable: true,
    ...overrides,
  };
}

function makeContext(overrides: Partial<ReadContext> = {}): ReadContext {
  return {
    connectionMode: "local_managed_hsd" as ConnectionMode,
    activeReadProvider: makeProvider(),
    fallbackActive: false,
    localNodeHealthy: true,
    walletAvailable: true,
    writeAllowed: true,
    ...overrides,
  };
}

describe("SourceBadge", () => {
  it("renders the provider label from the active context", () => {
    render(<SourceBadge context={makeContext()} />);
    expect(screen.getByText(/Local hsd/)).toBeInTheDocument();
  });

  it("falls back to a kind-derived label when no context is given", () => {
    render(<SourceBadge kind="external_hnsfans" />);
    expect(screen.getByText(/HNSFans/)).toBeInTheDocument();
  });

  it("defaults to local hsd when neither context nor kind is given", () => {
    render(<SourceBadge />);
    expect(screen.getByText(/Local hsd/)).toBeInTheDocument();
  });

  it("marks external providers as read-only", () => {
    render(
      <SourceBadge
        context={makeContext({
          activeReadProvider: makeProvider({
            kind: "external_hnsfans",
            label: "HNSFans",
            writeCapable: false,
          }),
          writeAllowed: false,
        })}
      />,
    );
    expect(screen.getByText(/read-only/)).toBeInTheDocument();
  });

  it("marks read-only when context disallows writes even for hsd", () => {
    render(
      <SourceBadge
        context={makeContext({
          activeReadProvider: makeProvider({ kind: "remote_hsd" }),
          writeAllowed: false,
        })}
      />,
    );
    expect(screen.getByText(/read-only/)).toBeInTheDocument();
  });

  it("does not mark read-only for a writable local provider", () => {
    render(<SourceBadge context={makeContext()} />);
    expect(screen.queryByText(/read-only/)).not.toBeInTheDocument();
  });

  it("prefers a custom remote label when no context label is present", () => {
    render(<SourceBadge kind="remote_hsd" remoteLabel="Home server" />);
    expect(screen.getByText(/Home server/)).toBeInTheDocument();
  });
});
