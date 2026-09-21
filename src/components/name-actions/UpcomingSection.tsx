/**
 * A section of the name-actions modal that belongs to a later stage.
 *
 * One muted line, no controls. The heading stays so the order of the
 * lifecycle is still visible — that was the whole argument for leaving these
 * sections on screen — while the buttons go, because a wall of disabled
 * controls is what made a user ask whether any of them were real.
 */
export function UpcomingSection({
  id,
  title,
  when,
}: {
  /** Stable key for tests — the heading is display copy and may be reworded. */
  id: string;
  title: string;
  when: string;
}) {
  return (
    <div className="text-xs text-gray-400" data-testid={`upcoming-section-${id}`}>
      <span className="font-medium text-gray-500">{title}</span> — {when}
    </div>
  );
}
