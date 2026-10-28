import { SETTINGS_SECTIONS, type SettingsSection } from "../../lib/settingsSections";
import { cn } from "../../lib/utils";
import { Icon } from "../ui/Icon";
import { Tooltip } from "../ui/Tooltip";

/** The Settings section list; a red dot marks a section holding an error that blocks Save. */
export function SettingsSectionNav({
  active,
  errors,
  onSelect,
}: {
  active: SettingsSection;
  errors: Partial<Record<SettingsSection, boolean>>;
  onSelect: (s: SettingsSection) => void;
}) {
  return (
    <nav
      aria-label="Settings sections"
      className="sticky top-0 w-52 shrink-0 flex flex-col gap-0.5 bg-white border border-gray-200 rounded-lg p-1.5"
    >
      {SETTINGS_SECTIONS.map((s) => (
        <button
          key={s.key}
          type="button"
          onClick={() => onSelect(s.key)}
          aria-current={active === s.key ? "page" : undefined}
          data-testid={`settings-section-${s.key}`}
          className={cn(
            "flex items-center gap-2.5 w-full text-left rounded-md px-2.5 py-2 text-sm cursor-pointer",
            active === s.key
              ? "bg-blue-50 text-blue-700 font-medium"
              : "text-gray-700 hover:bg-gray-100",
          )}
        >
          <Icon icon={s.icon} />
          <span className="flex-1">{s.label}</span>
          {errors[s.key] && (
            <Tooltip content="This section has an error to fix before saving">
              <span
                className="w-2 h-2 rounded-full bg-red-600"
                data-testid={`settings-section-error-${s.key}`}
              />
            </Tooltip>
          )}
        </button>
      ))}
    </nav>
  );
}
