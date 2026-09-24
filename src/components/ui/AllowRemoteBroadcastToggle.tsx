export interface AllowRemoteBroadcastToggleProps {
  /** Whether sending through a remote node is currently allowed. */
  checked: boolean;
  /** Called with the new value. */
  onChange: (checked: boolean) => void;
  /**
   * Render the explanatory line under the label. Settings shows it; the
   * onboarding step is already a page of explanation, so it does not.
   */
  showDescription?: boolean;
  /** Label size, matching the surrounding text. */
  size?: "sm" | "xs";
}

/**
 * The single "Allow sending via remote node" control, shared by the onboarding
 * Remote step and Settings.
 *
 * Both screens hand-rolled the same checkbox with the same label and the same
 * `data-testid`, which is two places for the wording of a safety opt-in to
 * drift apart and one testid that would match twice if a page ever rendered
 * both. What the toggle writes is the `allow_remote_broadcast` setting the
 * backend enforces in `broadcast_tx_draft` — the UI gate is not the guard, so
 * the two screens must at least agree on what they are asking.
 */
export function AllowRemoteBroadcastToggle({
  checked,
  onChange,
  showDescription = false,
  size = "sm",
}: AllowRemoteBroadcastToggleProps) {
  return (
    <label className={`flex items-center gap-2 ${size === "sm" ? "text-sm pt-2" : "text-xs"}`}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        data-testid="allow-remote-broadcast-checkbox"
      />
      <span>
        Allow sending via remote node
        {showDescription && (
          <div className="text-xs text-gray-500 font-normal">
            Off by default. Required to broadcast when chain source is Remote node.
          </div>
        )}
      </span>
    </label>
  );
}
