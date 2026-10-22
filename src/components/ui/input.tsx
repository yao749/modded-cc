import * as React from "react";
import { cn } from "@/lib/utils";

export type InputProps = React.InputHTMLAttributes<HTMLInputElement>;

export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, ...props }, ref) => (
    <input
      type={type}
      className={cn(
        "flex h-9 w-full px-3 py-1 text-sm file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-hidden disabled:cursor-not-allowed disabled:opacity-50 " +
          "plain:rounded-md plain:border plain:border-input plain:bg-transparent plain:shadow-xs plain:transition-colors plain:focus-visible:ring-1 plain:focus-visible:ring-ring " +
          "sketch:rounded-(--r-sketch-alt) sketch:border-[1.5px] sketch:border-(--line-2) sketch:bg-(--surface) sketch:transition-[border-color,box-shadow] sketch:hover:border-(--ink-4) sketch:focus-visible:border-(--stroke) sketch:focus-visible:shadow-[3px_3px_0_var(--fill-butter)] " +
          "win2k:border-0 win2k:bg-(--w-win) win2k:shadow-(--bevel-sunken) win2k:focus-visible:ring-0 win2k:focus-visible:shadow-(--bevel-sunken)",
        className,
      )}
      ref={ref}
      {...props}
    />
  ),
);
Input.displayName = "Input";
