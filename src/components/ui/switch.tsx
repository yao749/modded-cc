import * as React from "react";
import * as SwitchPrimitive from "@radix-ui/react-switch";
import { cn } from "@/lib/utils";

export const Switch = React.forwardRef<
  React.ElementRef<typeof SwitchPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitive.Root>
>(({ className, ...props }, ref) => (
  <SwitchPrimitive.Root
    className={cn(
      "peer inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent shadow-xs transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=unchecked]:bg-input " +
        // Win2000: 画成 13px 复选框 (白色凹陷方框 + 像素勾)
        "win2k:h-[13px] win2k:w-[13px] win2k:justify-center win2k:rounded-none win2k:border-0 win2k:shadow-(--bevel-sunken) win2k:data-[state=checked]:bg-(--w-win) win2k:data-[state=unchecked]:bg-(--w-win) win2k:focus-visible:ring-0 win2k:focus-visible:outline-1 win2k:focus-visible:outline-dotted win2k:focus-visible:outline-offset-2 win2k:focus-visible:outline-(--w-text)",
      className,
    )}
    {...props}
    ref={ref}
  >
    <SwitchPrimitive.Thumb
      className={cn(
        "pointer-events-none block h-4 w-4 rounded-full bg-background shadow-lg ring-0 transition-transform data-[state=checked]:translate-x-4 data-[state=unchecked]:translate-x-0 " +
          "win2k:h-[7px] win2k:w-[7px] win2k:rounded-none win2k:bg-(--w-text) win2k:shadow-none win2k:transition-none win2k:[mask:var(--w-check)] win2k:data-[state=checked]:translate-x-0 win2k:data-[state=unchecked]:opacity-0",
      )}
    />
  </SwitchPrimitive.Root>
));
Switch.displayName = SwitchPrimitive.Root.displayName;
