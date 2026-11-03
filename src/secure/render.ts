// Pure DOM rendering for the secure prompt window.
//
// Extracted from `main.ts` so it can be unit-tested under jsdom WITHOUT
// importing the Tauri window/IPC APIs. `main.ts` owns the Tauri bootstrap
// (fetching the request, deriving the prompt id, submitting over IPC) and
// delegates all DOM construction to `render` here. The secret values handled
// here still never cross into React state — this module is only loaded by the
// standalone secure window document.

import { writeText, readText } from "@tauri-apps/plugin-clipboard-manager";
import {
  clearClipboardIfUnchanged,
  scheduleClipboardClear,
  CLIPBOARD_CLEAR_MS,
} from "./clipboard-clear";

export interface PromptRequest {
  mode: "passphrase" | "passphrase_new" | "reveal" | "import" | "confirm";
  title: string;
  message: string;
  payload?: string | null;
  /** For `import`: what is being typed. Absent means a recovery phrase. */
  input?: "recovery_phrase" | "xpub" | null;
  details?: {
    rows?: { label: string; value: string }[];
    /** When set, Confirm stays disabled until exactly this text is typed
     * (a REVOKE asks for the name, a wallet delete for its label); the
     * backend checks the typed value too. */
    confirmText?: string | null;
    /** The Confirm button's text ("Delete wallet"); "Confirm & Sign" when
     * absent. */
    confirmLabel?: string | null;
  } | null;
}

export interface PromptResult {
  value: string | null;
  confirmed: boolean;
}

export const STYLE = `
  :root { color-scheme: light dark; }
  * { box-sizing: border-box; }
  body { margin: 0; font-family: system-ui, -apple-system, sans-serif; }
  .wrap { padding: 20px; display: flex; flex-direction: column; gap: 14px; height: 100vh; }
  h1 { font-size: 16px; margin: 0; }
  p.msg { font-size: 13px; color: #555; margin: 0; line-height: 1.4; }
  input, textarea { width: 100%; padding: 10px; font-size: 14px; border: 1px solid #bbb;
    border-radius: 8px; font-family: inherit; }
  textarea { resize: none; min-height: 96px; }
  .phrase { background: #f4f4f5; border: 1px solid #ddd; border-radius: 8px; padding: 14px;
    font-family: ui-monospace, monospace; font-size: 14px; line-height: 1.7; word-spacing: 4px;
    user-select: all; }
  .row { display: flex; gap: 8px; align-items: center; }
  .row.end { margin-top: auto; justify-content: flex-end; }
  .hint { font-size: 11px; color: #888; }
  label.chk { font-size: 13px; display: flex; gap: 8px; align-items: center; }
  .details { border: 1px solid #ddd; border-radius: 8px; overflow-y: auto; flex: 1 1 auto;
    min-height: 0; }
  .details .drow { display: flex; gap: 10px; padding: 8px 12px; font-size: 13px;
    border-top: 1px solid #eee; }
  .details .drow:first-child { border-top: none; }
  .details .dlabel { color: #777; min-width: 80px; max-width: 40%; flex: 0 0 auto; }
  .details .drow.warn .dvalue { color: #b45309; font-family: inherit; word-break: normal; }
  .details .dvalue { color: #111; word-break: break-all; white-space: pre-wrap;
    font-family: ui-monospace, monospace; }
  button { padding: 9px 16px; font-size: 14px; border-radius: 8px; border: 1px solid #bbb;
    background: #fff; cursor: pointer; }
  button.primary { background: #111; color: #fff; border-color: #111; }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
  .err { color: #c0392b; font-size: 12px; min-height: 14px; }
`;

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
  children: (Node | string)[] = [],
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  Object.assign(node, props);
  for (const c of children) node.append(c);
  return node;
}

/// Render `req` into `root`, wiring buttons to call `submit` with the user's
/// answer. Pure: no IPC, no window API — the caller supplies `submit`.
export function render(
  root: HTMLElement,
  req: PromptRequest,
  submit: (result: PromptResult) => void,
): void {
  const style = el("style", { textContent: STYLE });
  const wrap = el("div", { className: "wrap" });
  wrap.append(el("h1", { textContent: req.title }));
  wrap.append(el("p", { className: "msg", textContent: req.message }));
  const err = el("div", { className: "err" });
  // Runs once `wrap` is in the document, where it can be measured.
  let afterMount: (() => void) | null = null;

  if (req.mode === "passphrase" || req.mode === "passphrase_new") {
    const pw = el("input", { type: "password", placeholder: "Passphrase", autofocus: true });
    wrap.append(pw);
    let confirmInput: HTMLInputElement | null = null;
    if (req.mode === "passphrase_new") {
      confirmInput = el("input", { type: "password", placeholder: "Confirm passphrase" });
      wrap.append(confirmInput);
    }
    wrap.append(err);
    const ok = el("button", { className: "primary", textContent: "Continue" });
    const cancel = el("button", { textContent: "Cancel" });
    ok.onclick = () => {
      const v = pw.value;
      // The unlock prompt ("passphrase") requires a value; the create prompt
      // ("passphrase_new") allows blank to opt out of a passphrase.
      if (!v && req.mode === "passphrase") {
        err.textContent = "Passphrase is required.";
        return;
      }
      if (confirmInput && confirmInput.value !== v) {
        err.textContent = "Passphrases do not match.";
        return;
      }
      submit({ value: v, confirmed: true });
    };
    cancel.onclick = () => submit({ value: null, confirmed: false });
    pw.onkeydown = (e) => {
      if (e.key === "Enter" && req.mode === "passphrase") ok.click();
    };
    wrap.append(el("div", { className: "row end" }, [cancel, ok]));
  } else if (req.mode === "import") {
    const isXpub = req.input === "xpub";
    const ta = el("textarea", {
      placeholder: isXpub ? "Enter the account xpub" : "Enter your 12 or 24 word recovery phrase",
      autofocus: true,
    });
    wrap.append(ta, err);
    const ok = el("button", { className: "primary", textContent: "Import" });
    const cancel = el("button", { textContent: "Cancel" });
    ok.onclick = () => {
      const v = ta.value.trim().replace(/\s+/g, " ");
      if (!v) {
        err.textContent = isXpub ? "Account xpub is required." : "Recovery phrase is required.";
        return;
      }
      submit({ value: v, confirmed: true });
    };
    cancel.onclick = () => submit({ value: null, confirmed: false });
    wrap.append(el("div", { className: "row end" }, [cancel, ok]));
  } else if (req.mode === "reveal") {
    const phrase = el("div", { className: "phrase", textContent: req.payload ?? "" });
    wrap.append(phrase);
    const copy = el("button", { textContent: "Copy" });
    const clipboardHint = el("div", {
      className: "hint",
      textContent:
        "Clipboard clears when you press Done, or ~30s after copying while this window is open.",
    });
    let copied = false;
    copy.onclick = async () => {
      const value = req.payload ?? "";
      try {
        await writeText(value);
        copied = true;
        copy.textContent = "Copied";
        setTimeout(() => (copy.textContent = "Copy"), 1500);
        // Auto-clear: only wipes the clipboard if it still holds this exact
        // value by then (see clipboard-clear.ts for the compare/fallback
        // rationale) — so a seed phrase never lingers in the clipboard
        // indefinitely.
        scheduleClipboardClear(value, { readText, writeText }, CLIPBOARD_CLEAR_MS);
      } catch {
        /* clipboard unavailable; ignore */
      }
    };
    wrap.append(el("div", { className: "row" }, [copy]), clipboardHint);
    const chk = el("input", { type: "checkbox" });
    const lab = el("label", { className: "chk" }, [chk, "I have written down my recovery phrase"]);
    wrap.append(lab);
    const ok = el("button", { className: "primary", textContent: "Done", disabled: true });
    chk.onchange = () => (ok.disabled = !chk.checked);
    ok.onclick = async () => {
      ok.disabled = true;
      // Why now: the answer closes this window, which cancels the timer above.
      if (copied) await clearClipboardIfUnchanged(req.payload ?? "", { readText, writeText });
      submit({ value: null, confirmed: true });
    };
    wrap.append(el("div", { className: "row end" }, [ok]));
  } else if (req.mode === "confirm") {
    // Focusable so the arrow keys can scroll it to the end.
    const box = el("div", { className: "details", tabIndex: 0 });
    for (const r of req.details?.rows ?? []) {
      box.append(
        el("div", { className: r.label === "Warning" ? "drow warn" : "drow" }, [
          el("span", { className: "dlabel", textContent: r.label }),
          el("span", { className: "dvalue", textContent: r.value }),
        ]),
      );
    }
    wrap.append(box);
    const ok = el("button", {
      className: "primary",
      textContent: req.details?.confirmLabel || "Confirm & Sign",
    });
    const cancel = el("button", { textContent: "Cancel" });
    const expected = req.details?.confirmText ?? null;
    let typed: HTMLInputElement | null = null;
    // Rows the window cannot fit must be read before they can be confirmed: a
    // row can be long enough to push everything after it out of view.
    let readToEnd = false;
    const scrollHint = el("div", {
      className: "hint",
      textContent: "Scroll to the end of the details to continue.",
    });
    const refreshConfirm = () => {
      readToEnd ||= box.scrollTop + box.clientHeight >= box.scrollHeight - 2;
      scrollHint.hidden = readToEnd;
      ok.disabled = !readToEnd || (typed !== null && typed.value.trim() !== expected);
    };
    box.onscroll = refreshConfirm;
    afterMount = refreshConfirm;
    wrap.append(scrollHint);
    if (expected) {
      typed = el("input", {
        type: "text",
        placeholder: `Type ${expected} to confirm`,
        autocomplete: "off",
        spellcheck: false,
      });
      typed.oninput = refreshConfirm;
      wrap.append(typed);
    }
    ok.onclick = () => {
      submit({ value: typed ? typed.value.trim() : null, confirmed: true });
    };
    cancel.onclick = () => submit({ value: null, confirmed: false });
    wrap.append(el("div", { className: "row end" }, [cancel, ok]));
  } else {
    wrap.append(el("p", { className: "err", textContent: `Unknown prompt mode: ${req.mode}` }));
  }

  root.replaceChildren(style, wrap);
  afterMount?.();
}
