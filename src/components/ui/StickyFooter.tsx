import { cn } from "../../lib/utils";

interface StickyFooterProps {
  children: React.ReactNode;
  className?: string;
}

export function StickyFooter({ children, className }: StickyFooterProps) {
  return (
    <div
      className={cn(
        "sticky bottom-0 bg-white border-t border-gray-200 px-6 py-3 flex justify-between items-center z-10",
        className,
      )}
    >
      {children}
    </div>
  );
}
