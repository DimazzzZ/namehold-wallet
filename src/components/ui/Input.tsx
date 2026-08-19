import { forwardRef } from "react";
import { cn } from "../../lib/utils";

export const inputSizes = {
  sm: "px-2.5 py-1 text-xs",
  md: "px-3 py-1.5 text-sm",
};

interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  inputSize?: "sm" | "md";
}

export const Input = forwardRef<HTMLInputElement, InputProps>(function Input(
  { label, inputSize = "md", className, id, ...props },
  ref,
) {
  const inputId = id || label?.toLowerCase().replace(/\s+/g, "-");
  return (
    <div className="flex flex-col gap-1">
      {label && (
        <label htmlFor={inputId} className="text-sm font-medium text-gray-700">
          {label}
        </label>
      )}
      <input
        ref={ref}
        id={inputId}
        className={cn(
          "border border-gray-300 rounded focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent",
          inputSizes[inputSize],
          className,
        )}
        {...props}
      />
    </div>
  );
});
