import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";

// The secure window's `render` (extracted from main.ts) pulls in the clipboard
// plugin for `reveal` mode. Stub it so importing `render` doesn't reach for a
// real Tauri runtime; these tests don't exercise the copy button.
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn().mockResolvedValue(undefined),
  readText: vi.fn().mockResolvedValue(""),
}));

import { render, STYLE, type PromptRequest, type PromptResult } from "../render";

function mount() {
  const root = document.createElement("div");
  document.body.append(root);
  return root;
}

function buttonByText(root: HTMLElement, text: string): HTMLButtonElement {
  const btn = Array.from(root.querySelectorAll("button")).find((b) => b.textContent === text);
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

describe("secure render — confirm mode with typed confirmation", () => {
  const revokeReq: PromptRequest = {
    ...confirmReq,
    details: {
      rows: [
        { label: "Action", value: "Revoke name (burns it)" },
        { label: "Name", value: "alice" },
        { label: "Warning", value: "Revoking burns alice for good" },
      ],
      confirmText: "alice",
    },
  };

  it("keeps Confirm disabled until the name is typed exactly", () => {
    const root = mount();
    const submit = vi.fn<(r: PromptResult) => void>();
    render(root, revokeReq, submit);
    const ok = buttonByText(root, "Confirm & Sign");
    const input = root.querySelector('input[type="text"]') as HTMLInputElement;
    expect(input).not.toBeNull();
    expect(ok.disabled).toBe(true);
    ok.click();
    expect(submit).not.toHaveBeenCalled();

    input.value = "alic";
    input.dispatchEvent(new Event("input"));
    expect(ok.disabled).toBe(true);

    input.value = " alice ";
    input.dispatchEvent(new Event("input"));
    expect(ok.disabled).toBe(false);
    ok.click();
    expect(submit).toHaveBeenCalledWith({ value: "alice", confirmed: true });
  });

  it("marks warning rows so they stand out", () => {
    const root = mount();
    render(root, revokeReq, () => {});
    expect(root.querySelectorAll(".details .drow.warn")).toHaveLength(1);
  });

  it("names the Confirm button from confirmLabel (a wallet delete)", () => {
    const root = mount();
    const submit = vi.fn<(r: PromptResult) => void>();
    render(
      root,
      {
        ...confirmReq,
        title: "Delete wallet",
        details: {
          rows: [{ label: "Wallet", value: "Savings" }],
          confirmText: "Savings",
          confirmLabel: "Delete wallet",
        },
      },
      submit,
    );
    expect(Array.from(root.querySelectorAll("button")).map((b) => b.textContent)).toEqual([
      "Cancel",
      "Delete wallet",
    ]);
    const ok = buttonByText(root, "Delete wallet");
    expect(ok.disabled).toBe(true);
    const input = root.querySelector('input[type="text"]') as HTMLInputElement;
    input.value = "Savings";
    input.dispatchEvent(new Event("input"));
    ok.click();
    expect(submit).toHaveBeenCalledWith({ value: "Savings", confirmed: true });
  });

  it("keeps a row's line breaks visible (a signed message is shown as signed)", () => {
    expect(STYLE).toMatch(/\.details \.dvalue \{[^}]*white-space: pre-wrap/);
  });

  it("asks for nothing to be typed when no confirmText is set", () => {
    const root = mount();
    render(root, confirmReq, () => {});
    expect(root.querySelector('input[type="text"]')).toBeNull();
    expect(buttonByText(root, "Confirm & Sign").disabled).toBe(false);
  });
});

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

  it("a confirmation that signs nothing says so on its button", () => {
    const root = mount();
    const submit = vi.fn<(r: PromptResult) => void>();
    render(root, { ...confirmReq, details: { rows: [], confirmLabel: "Save settings" } }, submit);
    expect(root.textContent).not.toContain("Sign");
    buttonByText(root, "Save settings").click();
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

describe("secure render — confirm mode with more rows than fit", () => {
  // jsdom has no layout: give every element the sizes of an overflowing box.
  beforeEach(() => {
    vi.spyOn(Element.prototype, "scrollHeight", "get").mockReturnValue(1000);
    vi.spyOn(Element.prototype, "clientHeight", "get").mockReturnValue(400);
  });
  afterEach(() => vi.restoreAllMocks());

  const scrollTo = (box: HTMLElement, top: number) => {
    box.scrollTop = top;
    box.dispatchEvent(new Event("scroll"));
  };

  it("keeps Confirm disabled and says why until the rows are scrolled to the end", () => {
    const root = mount();
    const submit = vi.fn<(r: PromptResult) => void>();
    render(root, confirmReq, submit);
    const ok = buttonByText(root, "Confirm & Sign");
    const box = root.querySelector(".details") as HTMLElement;
    const hint = root.querySelector(".hint") as HTMLElement;
    expect(ok.disabled).toBe(true);
    expect(hint.hidden).toBe(false);
    expect(hint.textContent).toContain("Scroll to the end");

    scrollTo(box, 300);
    expect(ok.disabled).toBe(true);
    ok.click();
    expect(submit).not.toHaveBeenCalled();

    scrollTo(box, 600);
    expect(ok.disabled).toBe(false);
    expect(hint.hidden).toBe(true);
    ok.click();
    expect(submit).toHaveBeenCalledWith({ value: null, confirmed: true });
  });

  it("stays open once the end was seen, even scrolled back up", () => {
    const root = mount();
    render(root, confirmReq, vi.fn());
    const ok = buttonByText(root, "Confirm & Sign");
    const box = root.querySelector(".details") as HTMLElement;
    scrollTo(box, 600);
    scrollTo(box, 0);
    expect(ok.disabled).toBe(false);
  });

  it("needs both the scroll and the typed text when both apply", () => {
    const root = mount();
    render(
      root,
      { ...confirmReq, details: { ...confirmReq.details, confirmText: "alice" } },
      vi.fn(),
    );
    const ok = buttonByText(root, "Confirm & Sign");
    const box = root.querySelector(".details") as HTMLElement;
    const input = root.querySelector('input[type="text"]') as HTMLInputElement;
    input.value = "alice";
    input.dispatchEvent(new Event("input"));
    expect(ok.disabled).toBe(true);
    scrollTo(box, 600);
    expect(ok.disabled).toBe(false);
    input.value = "alic";
    input.dispatchEvent(new Event("input"));
    expect(ok.disabled).toBe(true);
  });
});

describe("secure render — confirm mode when the rows fit", () => {
  it("asks for no scrolling", () => {
    const root = mount();
    render(root, confirmReq, vi.fn());
    expect(buttonByText(root, "Confirm & Sign").disabled).toBe(false);
    expect((root.querySelector(".hint") as HTMLElement).hidden).toBe(true);
  });
});

describe("secure render — reveal mode", () => {
  const revealReq: PromptRequest = {
    mode: "reveal",
    title: "Back up",
    message: "Write it down.",
    payload: "the phrase",
  };
  const tick = () => new Promise((r) => setTimeout(r));

  afterEach(() => vi.clearAllMocks());

  function done(root: HTMLElement) {
    const chk = root.querySelector('input[type="checkbox"]') as HTMLInputElement;
    chk.checked = true;
    chk.dispatchEvent(new Event("change"));
    buttonByText(root, "Done").click();
  }

  // The window closes on the answer, which cancels the 30 s clear.
  it("clears a copied phrase from the clipboard before Done answers", async () => {
    const root = mount();
    const order: string[] = [];
    vi.mocked(readText).mockResolvedValueOnce("the phrase");
    const submit = vi.fn(() => void order.push("submit"));
    render(root, revealReq, submit);
    buttonByText(root, "Copy").click();
    await tick();
    vi.mocked(writeText).mockImplementationOnce(async () => void order.push("clear"));

    done(root);
    await tick();

    expect(writeText).toHaveBeenLastCalledWith("");
    expect(order).toEqual(["clear", "submit"]);
    expect(submit).toHaveBeenCalledWith({ value: null, confirmed: true });
  });

  it("leaves the clipboard alone when the phrase was not copied or was replaced", async () => {
    const root = mount();
    const submit = vi.fn();
    render(root, revealReq, submit);
    done(root);
    await tick();
    expect(readText).not.toHaveBeenCalled();
    expect(writeText).not.toHaveBeenCalled();
    expect(submit).toHaveBeenCalledTimes(1);

    const replaced = mount();
    vi.mocked(readText).mockResolvedValueOnce("something else");
    render(replaced, revealReq, submit);
    buttonByText(replaced, "Copy").click();
    await tick();
    done(replaced);
    await tick();
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith("the phrase");
  });
});

describe("secure render — extraction smoke test", () => {
  it("renders passphrase mode with a required-value guard", () => {
    const root = mount();
    const submit = vi.fn<(r: PromptResult) => void>();
    render(root, { mode: "passphrase", title: "Unlock", message: "Enter passphrase" }, submit);
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

describe("secure render — import mode input kind", () => {
  const base = { mode: "import" as const, title: "Import", message: "Enter it." };

  it("asks for a recovery phrase by default", () => {
    const root = mount();
    render(root, base, () => {});
    expect(root.querySelector("textarea")?.placeholder).toBe(
      "Enter your 12 or 24 word recovery phrase",
    );
    buttonByText(root, "Import").click();
    expect(root.querySelector(".err")?.textContent).toBe("Recovery phrase is required.");
  });

  it("shows an xpub placeholder and error for the account xpub import", () => {
    const root = mount();
    const submit = vi.fn<(r: PromptResult) => void>();
    render(root, { ...base, input: "xpub" }, submit);
    expect(root.querySelector("textarea")?.placeholder).toBe("Enter the account xpub");
    buttonByText(root, "Import").click();
    expect(submit).not.toHaveBeenCalled();
    expect(root.querySelector(".err")?.textContent).toBe("Account xpub is required.");
  });
});
