import { Dialog } from "./ui/Dialog";
import { ReleaseNotes } from "./ReleaseNotes";

/**
 * Modal displaying the release notes for an available app update.
 * Renders the GitHub release body (Markdown) in a scrollable dialog.
 */
export function WhatsNewModal({
  open,
  onClose,
  version,
  notes,
}: {
  open: boolean;
  onClose: () => void;
  version: string;
  notes: string;
}) {
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`What's new in v${version}`}
      className="max-w-2xl"
    >
      <div className="space-y-2 text-sm max-h-[70vh] overflow-y-auto" data-testid="whats-new-modal">
        <ReleaseNotes notes={notes} />
      </div>
    </Dialog>
  );
}
