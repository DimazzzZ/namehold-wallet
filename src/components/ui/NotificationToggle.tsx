import { useEffect, useState, type ReactNode } from "react";
import {
  checkNotificationPermission,
  requestNotificationPermission,
  type PermissionStatus,
} from "../../lib/notifications";
import { boolToSetting, settingToBool } from "../../lib/settingsBool";
import type { Settings } from "../../types";

export interface NotificationToggleProps {
  /** The `Settings` key this toggle writes ("true" / "false"). */
  settingKey: keyof Settings;
  /** Checkbox label. */
  label: string;
  /** testid for the checkbox. */
  testId: string;
  /** testid for the "OS notifications are blocked" notice. */
  deniedTestId: string;
  /**
   * What the user loses while OS notifications are blocked. Each kind of alert
   * loses something different, so the sentence is the caller's; the framing
   * around it is not.
   */
  deniedConsequence: ReactNode;
  /** The settings form and its updater, as the Settings screen holds them. */
  form: Record<string, string>;
  updateField: (key: string, value: string) => void;
  /** Extra fields shown only while the toggle is on — lead times and the like. */
  children?: ReactNode;
}

/**
 * One notification opt-in: the checkbox, the OS permission request it triggers,
 * and the three notices that follow from the answer.
 *
 * The permission request has to originate from the user's click — macOS
 * silently denies one that does not — so it lives here beside the checkbox
 * rather than in the form's Save. The enabled flag itself is an ordinary form
 * field, saved with every other setting.
 *
 * Three sections had this written out in full, identical but for the setting
 * key, the label and one sentence. That is three places to fix a permission
 * bug and three chances for the notices to drift apart.
 */
export function NotificationToggle({
  settingKey,
  label,
  testId,
  deniedTestId,
  deniedConsequence,
  form,
  updateField,
  children,
}: NotificationToggleProps) {
  const [permission, setPermission] = useState<PermissionStatus | null>(null);
  const [requesting, setRequesting] = useState(false);
  const enabled = settingToBool(form[settingKey]);

  useEffect(() => {
    checkNotificationPermission().then(setPermission);
  }, []);

  const onToggle = async (checked: boolean) => {
    updateField(settingKey, boolToSetting(checked));
    // Turning it off needs no permission, and asking then would be a prompt
    // the user did not invite.
    if (!checked) return;
    setRequesting(true);
    try {
      setPermission(await requestNotificationPermission());
    } finally {
      setRequesting(false);
    }
  };

  return (
    <div className="space-y-3">
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => onToggle(e.target.checked)}
          data-testid={testId}
        />
        {label}
      </label>

      {enabled && (
        <>
          {permission === "denied" && (
            <div
              className="text-xs text-amber-600 bg-amber-50 border border-amber-200 rounded p-2"
              data-testid={deniedTestId}
            >
              OS notifications are blocked for this app. Enable them in your system notification
              settings — {deniedConsequence}
            </div>
          )}
          {permission === "unsupported" && (
            <div className="text-xs text-gray-500">
              OS notifications aren&apos;t available outside the desktop app.
            </div>
          )}
          {requesting && <div className="text-xs text-gray-500">Requesting permission…</div>}
          {children}
        </>
      )}
    </div>
  );
}
