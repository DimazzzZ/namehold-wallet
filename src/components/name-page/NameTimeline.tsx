import { Check, X } from "lucide-react";
import { Icon } from "../ui/Icon";
import { Tooltip } from "../ui/Tooltip";
import { cn } from "../../lib/utils";
import type { TimelineStep } from "../../lib/nameStatus";

const CIRCLE: Record<TimelineStep["kind"], string> = {
  done: "bg-gray-100 border border-gray-300",
  current: "bg-blue-600 ring-4 ring-blue-100",
  next: "bg-white border-2 border-dashed border-gray-300",
  lost: "bg-red-100 border border-red-300",
  na: "bg-white border border-gray-200",
};
const LABEL: Record<TimelineStep["kind"], string> = {
  done: "text-gray-500",
  current: "text-blue-700 font-semibold",
  next: "text-gray-900 font-medium",
  lost: "text-red-800 font-semibold",
  na: "text-gray-300",
};
const SUB: Record<TimelineStep["kind"], string> = {
  done: "text-gray-400",
  current: "text-gray-600",
  next: "text-gray-600",
  lost: "text-red-800",
  na: "text-gray-300",
};

/** The name's lifecycle as six steps, from `nameTimeline()`. */
export function NameTimeline({ steps, urgent }: { steps: TimelineStep[]; urgent?: boolean }) {
  const inset = `${100 / (steps.length * 2)}%`;
  return (
    <div
      className="bg-white border border-gray-200 rounded-lg px-6 pt-[18px] pb-4"
      data-testid="name-timeline"
    >
      <ol className="relative flex">
        <span
          aria-hidden
          className="absolute top-[9px] h-0.5 bg-gray-200"
          style={{ left: inset, right: inset }}
        />
        {steps.map((step, i) => {
          const redNext = urgent && step.kind === "next";
          const circle = (
            <span
              className={cn(
                "relative z-10 flex h-5 w-5 items-center justify-center rounded-full",
                redNext ? "bg-white border-2 border-dashed border-red-300" : CIRCLE[step.kind],
              )}
            >
              {step.kind === "done" && <Icon icon={Check} size={12} className="text-gray-500" />}
              {step.kind === "lost" && <Icon icon={X} size={12} className="text-red-700" />}
              {step.kind === "current" && <span className="h-1.5 w-1.5 rounded-full bg-white" />}
            </span>
          );
          return (
            <li
              key={i}
              className="flex-1 min-w-0 flex flex-col items-center text-center gap-1.5"
              data-testid={`timeline-step-${step.kind}`}
            >
              {step.kind === "na" ? (
                <Tooltip content="Not for this wallet">{circle}</Tooltip>
              ) : (
                circle
              )}
              <span
                className={cn(
                  "text-[13px]",
                  redNext ? "text-red-700 font-medium" : LABEL[step.kind],
                )}
                aria-current={step.kind === "current" ? "step" : undefined}
              >
                {step.label}
              </span>
              {step.sub && (
                <span className={cn("text-xs tabular-nums", SUB[step.kind])}>{step.sub}</span>
              )}
            </li>
          );
        })}
      </ol>
    </div>
  );
}
