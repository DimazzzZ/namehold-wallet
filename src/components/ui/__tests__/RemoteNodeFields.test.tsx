import { describe, it, expect, vi } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent } from "@testing-library/react";

import { RemoteNodeFields } from "../RemoteNodeFields";
import type { NodeConnectionCheckState } from "../../../hooks/useNodeConnectionCheck";

function probeState(over: Partial<NodeConnectionCheckState> = {}): NodeConnectionCheckState {
  return {
    testing: false,
    result: null,
    error: null,
    ok: false,
    run: vi.fn().mockResolvedValue(undefined),
    reset: vi.fn(),
    ...over,
  };
}

function renderFields(props: Partial<Parameters<typeof RemoteNodeFields>[0]> = {}) {
  const probe = props.probe ?? probeState();
  render(
    <RemoteNodeFields
      url=""
      apiKey=""
      onUrlChange={vi.fn()}
      onApiKeyChange={vi.fn()}
      {...props}
      probe={probe}
    />,
  );
  return probe;
}

describe("RemoteNodeFields", () => {
  it("defaults to the Settings testids so only onboarding has to override them", () => {
    renderFields();
    expect(screen.getByTestId("node-rpc-url-input")).toBeInTheDocument();
    expect(screen.getByTestId("node-rpc-api-key-input")).toBeInTheDocument();
    expect(screen.getByTestId("test-connection-button")).toHaveTextContent("Test connection");
  });

  it("labels each input independently", () => {
    renderFields({ urlLabel: "Node RPC URL (sending)", apiKeyLabel: "Node RPC API key" });
    expect(screen.getByText("Node RPC URL (sending)")).toBeInTheDocument();
    expect(screen.getByText("Node RPC API key")).toBeInTheDocument();
  });

  it("renders a labelless field pair when no labels are given", () => {
    renderFields({ urlPlaceholder: "https://node.example.com:12037" });
    expect(screen.queryByText("Node RPC API key")).not.toBeInTheDocument();
    expect(screen.getByPlaceholderText("https://node.example.com:12037")).toBeInTheDocument();
  });

  it("reports edits to each field separately", () => {
    const onUrlChange = vi.fn();
    const onApiKeyChange = vi.fn();
    renderFields({ onUrlChange, onApiKeyChange });

    fireEvent.change(screen.getByTestId("node-rpc-url-input"), {
      target: { value: "http://127.0.0.1:12037" },
    });
    fireEvent.change(screen.getByTestId("node-rpc-api-key-input"), {
      target: { value: "sekrit" },
    });

    expect(onUrlChange).toHaveBeenCalledWith("http://127.0.0.1:12037");
    expect(onApiKeyChange).toHaveBeenCalledWith("sekrit");
  });

  it("invalidates a stale probe result itself, so no caller has to remember", () => {
    const probe = renderFields({ probe: probeState({ ok: true }) });

    fireEvent.change(screen.getByTestId("node-rpc-url-input"), {
      target: { value: "http://127.0.0.1:12037" },
    });
    expect(probe.reset).toHaveBeenCalledTimes(1);

    fireEvent.change(screen.getByTestId("node-rpc-api-key-input"), {
      target: { value: "sekrit" },
    });
    expect(probe.reset).toHaveBeenCalledTimes(2);
  });

  it("probes the current url and key when Test connection is clicked", () => {
    const probe = renderFields({ url: "http://127.0.0.1:12037", apiKey: "sekrit" });
    fireEvent.click(screen.getByTestId("test-connection-button"));
    expect(probe.run).toHaveBeenCalledWith("http://127.0.0.1:12037", "sekrit");
  });

  it("disables the button and swaps its label while a probe is in flight", () => {
    renderFields({ probe: probeState({ testing: true }) });
    const button = screen.getByTestId("test-connection-button");
    expect(button).toBeDisabled();
    expect(button).toHaveTextContent("Testing…");
  });

  it("shows the probe outcome next to the button", () => {
    renderFields({
      probe: probeState({
        ok: true,
        result: {
          reachable: true,
          height: 42,
          headers: 42,
          synced: true,
          network: "regtest",
          networkMatches: null,
          error: null,
        },
      }),
    });
    expect(screen.getByTestId("connection-success")).toHaveTextContent("height 42");
  });

  it("shows a probe failure instead of an outcome", () => {
    renderFields({ probe: probeState({ error: "Node unreachable" }) });
    expect(screen.getByTestId("connection-error")).toHaveTextContent("Node unreachable");
    expect(screen.queryByTestId("connection-success")).not.toBeInTheDocument();
  });

  it("lays the button and status out in a row by default, stacked on request", () => {
    const { unmount } = render(
      <RemoteNodeFields
        url=""
        apiKey=""
        onUrlChange={vi.fn()}
        onApiKeyChange={vi.fn()}
        probe={probeState()}
      />,
    );
    expect(screen.getByTestId("test-connection-button").parentElement).toHaveClass(
      "flex",
      "items-center",
      "gap-2",
    );
    unmount();

    renderFields({ actionsLayout: "stack" });
    expect(screen.getByTestId("test-connection-button").parentElement).toHaveClass("space-y-2");
  });
});
