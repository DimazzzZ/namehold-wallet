// Secure prompt entry point.
//
// This is a STANDALONE document loaded into a Rust-owned `secure-prompt-*`
// window. It is intentionally NOT part of the React app bundle: the React app
// never imports this file, and the secret values handled here (passphrases,
// mnemonics) never cross into React state. Communication is window <-> Rust
// only, via the `secure_prompt_fetch` / `secure_prompt_submit` commands.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { render, type PromptRequest, type PromptResult } from "./render";

// The window label is `secure-prompt-<id>`; derive the id from it. No IPC
// needed — the label is available synchronously from window metadata.
const promptId = getCurrentWindow().label.replace(/^secure-prompt-/, "");
const root = document.getElementById("root")!;

function submit(result: PromptResult) {
  // Fire-and-forget: the backend closes the window on receipt.
  invoke("secure_prompt_submit", { promptId, result }).catch(() => {});
}

async function main() {
  try {
    const req = await invoke<PromptRequest>("secure_prompt_fetch", { promptId });
    render(root, req, submit);
  } catch (e) {
    root.textContent = `Secure prompt unavailable: ${String(e)}`;
  }
}

main();
