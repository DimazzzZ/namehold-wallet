import type { LucideIcon } from "lucide-react";
import { Archive, Bell, CircleArrowUp, Plug, Server, SlidersHorizontal } from "lucide-react";

export type SettingsSection =
  "connections" | "node" | "notifications" | "import" | "updates" | "advanced";

export const SETTINGS_SECTIONS: { key: SettingsSection; label: string; icon: LucideIcon }[] = [
  { key: "connections", label: "Connections", icon: Plug },
  { key: "node", label: "Node & tray", icon: Server },
  { key: "notifications", label: "Notifications", icon: Bell },
  { key: "import", label: "Backup & import", icon: Archive },
  { key: "updates", label: "Updates", icon: CircleArrowUp },
  { key: "advanced", label: "Advanced", icon: SlidersHorizontal },
];

/** ?section= → a section; "backup" is an alias of "import"; unknown → connections. */
export function parseSettingsSection(raw: string | null): SettingsSection {
  if (raw === "backup") return "import";
  return SETTINGS_SECTIONS.some((s) => s.key === raw) ? (raw as SettingsSection) : "connections";
}

/** "/settings", or "/settings?section=<key>". */
export function settingsPath(section?: SettingsSection): string {
  return section ? `/settings?section=${section}` : "/settings";
}
