/**
 * Browser-safe wrapper around `@tauri-apps/plugin-notification`'s OS
 * permission handshake (I1 — deadline scanner notifications).
 *
 * The actual notification DISPATCH happens on the Rust side (the background
 * scanner in `src-tauri/src/commands/deadlines.rs`, via
 * `tauri_plugin_notification::NotificationExt`) — this module only handles
 * the one thing that must happen from a user gesture: requesting OS
 * permission when the user turns the Settings toggle on. In a plain browser
 * (no Tauri) this is a no-op that reports "denied", since there is no OS
 * notification center to ask.
 */
import { isTauri } from "./runtime";

/**
 * "prompt" = permission hasn't been decided yet (the user has never been
 * asked, or the OS hasn't recorded a decision) — distinct from "denied" (the
 * user, or the OS, actively said no). See `checkNotificationPermission` for
 * why this distinction needs its own state rather than folding into
 * "denied".
 */
export type PermissionStatus = "granted" | "denied" | "prompt" | "unsupported";

/**
 * Current permission state, without prompting. Never throws — an
 * unavailable notification backend (e.g. no `window.Notification` in an
 * embedded/odd webview) degrades to "unsupported" rather than crashing the
 * Settings page.
 *
 * Review Minor 8: `@tauri-apps/plugin-notification`'s `isPermissionGranted()`
 * is TYPED `Promise<boolean>`, but its underlying Tauri command
 * (`plugin:notification|is_permission_granted`) actually resolves an
 * `Option<bool>` — `None` (JSON `null`) for "not yet determined" (macOS
 * `UNUserNotificationCenter` `notDetermined`, or any platform's
 * Prompt/PromptWithRationale state), separately from `Some(false)` for an
 * explicit denial. A naive `(await isPermissionGranted()) ? "granted" :
 * "denied"` silently coerces that `null` to `false` (nullish is falsy in
 * JS), so a user who has simply never been asked sees the SAME "OS
 * notifications are blocked" warning as one who explicitly said no. This
 * reads the raw resolved value instead of trusting the (misleading) declared
 * type, so "not yet determined" surfaces as "prompt", not "denied". (On the
 * desktop build of this specific plugin version, the Rust side currently
 * always reports Granted regardless of the real OS state — see
 * `tauri-plugin-notification`'s `desktop.rs` — so `null`/"prompt" is mainly
 * reachable on mobile targets today; handling it correctly here costs
 * nothing and avoids the false-positive warning if that ever changes.)
 */
export async function checkNotificationPermission(): Promise<PermissionStatus> {
  if (!isTauri()) return "unsupported";
  try {
    const { isPermissionGranted } = await import("@tauri-apps/plugin-notification");
    const granted = (await isPermissionGranted()) as unknown as boolean | null | undefined;
    if (granted === true) return "granted";
    if (granted === false) return "denied";
    return "prompt"; // null/undefined: not yet determined, NOT denied
  } catch {
    return "unsupported";
  }
}

/**
 * Prompt the OS permission dialog if not already decided. Must be called
 * from a user gesture (the Settings toggle) — macOS silently denies
 * permission requests made without one. Never throws (see
 * `checkNotificationPermission`).
 */
export async function requestNotificationPermission(): Promise<PermissionStatus> {
  if (!isTauri()) return "unsupported";
  try {
    const { isPermissionGranted, requestPermission } = await import(
      "@tauri-apps/plugin-notification"
    );
    if (await isPermissionGranted()) return "granted";
    const result = await requestPermission();
    return result === "granted" ? "granted" : "denied";
  } catch {
    return "unsupported";
  }
}
