import { describe, it, expect, vi } from "vitest";

// The secure window's `render` (extracted from main.ts) pulls in the clipboard
// plugin for `reveal` mode. Stub it so importing `render` doesn't reach for a
// real Tauri runtime; these tests don't exercise the copy button.
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn().mockResolvedValue(undefined),
  readText: vi.fn().mockResolvedValue(""),
}));

import { render, type PromptRequest, type PromptResult } from "../render";

function mount() {
  const root = document.createElement("div");
  document.body.append(root);
  return root;
}

function buttonByText(root: HTMLElement, text: string): HTMLButtonElement {
  const btn = Array.from(root.querySelectorAll("button")).find(
    (b) => b.textContent === text,
  );
  if (!btn) throw new Error(`button "${text}" not found`);
  return btn as HTMLButtonElement;
}

const confirmReq: PromptRequest = {
  mode: "confirm",
  title: "Confirm transaction",
  message: "Review these details.",
  details: {
    rows: [
      { label: "Action", value: "Send HNS" },
      { label: "To", value: "hs1qexample" },
      { label: "Amount", value: "1.500000 HNS" },
    ],
  },
};

describe("secure render — confirm mode", () => {
  it("renders confirm mode with detail rows", () => {
    const root = mount();
    render(root, confirmReq, () => {});
    const rows = root.querySelectorAll(".details .drow");
    expect(rows).toHaveLength(3);
    expect(root.textContent).toContain("Send HNS");
    expect(root.textContent).toContain("hs1qexample");
    expect(root.textContent).toContain("1.500000 HNS");
  });

  it("Confirm button calls submit with { confirmed: true }", () => {
    const root = mount();
    const submit = vi.fn<(r: PromptResult) => void>();
    render(root, confirmReq, submit);
    buttonByText(root, "Confirm & Sign").click();
    expect(submit).toHaveBeenCalledWith({ value: null, confirmed: true });
  });

  it("Cancel button calls submit with { confirmed: false }", () => {
    const root = mount();
    const submit = vi.fn<(r: PromptResult) => void>();
    render(root, confirmReq, submit);
    buttonByText(root, "Cancel").click();
    expect(submit).toHaveBeenCalledWith({ value: null, confirmed: false });
  });

  it("renders zero rows without crashing when details is empty", () => {
    const root = mount();
    render(root, { ...confirmReq, details: { rows: [] } }, () => {});
    expect(root.querySelectorAll(".details .drow")).toHaveLength(0);
    // Buttons still render so the user can cancel.
    expect(buttonByText(root, "Cancel")).toBeTruthy();
  });

  it("escapes untrusted row text (renders as textContent, not innerHTML)", () => {
    const root = mount();
    const malicious = "<script>alert(1)</script>";
    render(
      root,
      { ...confirmReq, details: { rows: [{ label: "To", value: malicious }] } },
      () => {},
    );
    // No actual <script> element is created — the value is inert text.
    expect(root.querySelector("script")).toBeNull();
    const dvalue = root.querySelector(".details .dvalue");
    expect(dvalue?.textContent).toBe(malicious);
  });
});

describe("secure render — extraction smoke test", () => {
  it("renders passphrase mode with a required-value guard", () => {
    const root = mount();
    const submit = vi.fn<(r: PromptResult) => void>();
    render(
      root,
      { mode: "passphrase", title: "Unlock", message: "Enter passphrase" },
      submit,
    );
    // Empty passphrase is rejected (no submit, error shown).
    buttonByText(root, "Continue").click();
    expect(submit).not.toHaveBeenCalled();
    expect(root.querySelector(".err")?.textContent).toContain("required");

    // A typed passphrase submits.
    const pw = root.querySelector<HTMLInputElement>('input[type="password"]')!;
    pw.value = "hunter2";
    buttonByText(root, "Continue").click();
    expect(submit).toHaveBeenCalledWith({ value: "hunter2", confirmed: true });
  });
});
