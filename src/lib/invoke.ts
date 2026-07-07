import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { isBrowser } from "./runtime";
import { mockInvoke } from "./webqa-mock";

export async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (isBrowser()) {
    return mockInvoke<T>(cmd, args);
  }

  return tauriInvoke<T>(cmd, args);
}
