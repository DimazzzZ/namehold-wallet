import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  clearClipboardIfUnchanged,
  scheduleClipboardClear,
  CLIPBOARD_CLEAR_MS,
} from "../clipboard-clear";

// Seed-phrase clipboard auto-clear (Task 14 / S5). Kept in a side-effect-free
// module specifically so this logic is testable — `main.ts` itself bootstraps
// a whole DOM/Tauri window on import and has no test harness (see the module
// doc comment in clipboard-clear.ts).

describe("clearClipboardIfUnchanged", () => {
  it("clears the clipboard when it still holds the copied value", async () => {
    const readText = vi.fn().mockResolvedValue("my seed phrase");
    const writeText = vi.fn().mockResolvedValue(undefined);

    await clearClipboardIfUnchanged("my seed phrase", { readText, writeText });

    expect(writeText).toHaveBeenCalledWith("");
  });

  it("leaves the clipboard alone when the user copied something else in the meantime", async () => {
    const readText = vi.fn().mockResolvedValue("something else entirely");
    const writeText = vi.fn().mockResolvedValue(undefined);

    await clearClipboardIfUnchanged("my seed phrase", { readText, writeText });

    expect(writeText).not.toHaveBeenCalled();
  });

  it("falls back to an unconditional clear when the clipboard can't be read back", async () => {
    const readText = vi.fn().mockRejectedValue(new Error("read not permitted"));
    const writeText = vi.fn().mockResolvedValue(undefined);

    await clearClipboardIfUnchanged("my seed phrase", { readText, writeText });

    expect(writeText).toHaveBeenCalledWith("");
  });

  it("never throws if the write itself fails (clipboard unavailable)", async () => {
    const readText = vi.fn().mockResolvedValue("my seed phrase");
    const writeText = vi.fn().mockRejectedValue(new Error("clipboard unavailable"));

    await expect(
      clearClipboardIfUnchanged("my seed phrase", { readText, writeText }),
    ).resolves.toBeUndefined();
  });
});

describe("scheduleClipboardClear", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("does not clear before the delay elapses", async () => {
    const readText = vi.fn().mockResolvedValue("secret");
    const writeText = vi.fn().mockResolvedValue(undefined);

    scheduleClipboardClear("secret", { readText, writeText }, CLIPBOARD_CLEAR_MS);
    await vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_MS - 1);

    expect(writeText).not.toHaveBeenCalled();
  });

  it("clears once the delay elapses", async () => {
    const readText = vi.fn().mockResolvedValue("secret");
    const writeText = vi.fn().mockResolvedValue(undefined);

    scheduleClipboardClear("secret", { readText, writeText }, CLIPBOARD_CLEAR_MS);
    await vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_MS);

    expect(writeText).toHaveBeenCalledWith("");
  });
});
