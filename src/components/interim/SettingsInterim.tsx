// Interim (shell-v2-2026-10-28). Replaced by settings-sections-2026-10-28; delete when that lands.
import { useSearchParams } from "react-router-dom";
import { MigrationWorkspace } from "../MigrationWorkspace";
import { Settings } from "../Settings";
import { Tabs } from "../ui/Tabs";

/** /settings until the sectioned Settings lands: the whole page | the Namebase import. */
export function SettingsInterim() {
  const [params, setParams] = useSearchParams();
  const section = params.get("section");
  const tab = section === "import" || section === "backup" ? "import" : "settings";
  return (
    <div className="flex flex-col gap-5">
      <Tabs
        tabs={[
          { key: "settings", label: "Settings" },
          { key: "import", label: "Import from Namebase" },
        ]}
        active={tab}
        onChange={(key) =>
          setParams(key === "import" ? { section: "import" } : {}, { replace: true })
        }
      />
      {tab === "import" ? <MigrationWorkspace /> : <Settings />}
    </div>
  );
}
