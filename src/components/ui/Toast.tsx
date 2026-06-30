import { useUiStore } from "../../stores/ui";
import { cn } from "../../lib/utils";

export function Toast() {
  const { toastMessage, toastType, clearToast } = useUiStore();
  if (!toastMessage) return null;
  const colors = {
    info: "bg-blue-600",
    error: "bg-red-600",
    success: "bg-green-600",
  };
  return (
    <div
      className={cn(
        "fixed bottom-4 right-4 z-50 px-4 py-2 rounded text-white text-sm shadow-lg cursor-pointer",
        colors[toastType],
      )}
      onClick={clearToast}
    >
      {toastMessage}
    </div>
  );
}
