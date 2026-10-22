import * as React from "react";
import * as SelectPrimitive from "@radix-ui/react-select";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

export const Select = SelectPrimitive.Root;
export const SelectGroup = SelectPrimitive.Group;
export const SelectValue = SelectPrimitive.Value;

export const SelectLabel = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Label>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Label>
>(({ className, ...props }, ref) => (
  <SelectPrimitive.Label
    ref={ref}
    className={cn(
      "px-2 py-1.5 text-xs font-medium text-(--ink-4)",
      className,
    )}
    {...props}
  />
));
SelectLabel.displayName = SelectPrimitive.Label.displayName;

export const SelectTrigger = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Trigger>
>(({ className, children, ...props }, ref) => (
  <SelectPrimitive.Trigger
    ref={ref}
    className={cn(
      // 与全局 .select / .input 同视觉, 颜色走项目自有 token 以跟随 .dark 与主题; 外形按画风分两套
      "flex w-full items-center justify-between gap-2 whitespace-nowrap border-(--line-2) bg-(--surface) px-3 py-2 text-left text-[13px] text-(--ink) transition-[border-color,box-shadow] duration-150 focus:outline-hidden disabled:cursor-not-allowed disabled:bg-(--surface-2) disabled:text-(--ink-4) [&>span]:line-clamp-1 " +
        "plain:rounded-(--r-sm) plain:border plain:focus:border-(--ink-2) plain:focus:shadow-[0_0_0_3px_rgba(0,0,0,0.04)] plain:dark:focus:shadow-[0_0_0_3px_rgba(255,255,255,0.10)] " +
        "sketch:rounded-(--r-sketch-alt) sketch:border-[1.5px] sketch:hover:border-(--ink-4) sketch:focus:border-(--stroke) sketch:focus:shadow-[3px_3px_0_var(--fill-butter)] " +
        "win2k:border-0 win2k:bg-(--w-win) win2k:py-[3px] win2k:pl-[6px] win2k:pr-[2px] win2k:shadow-(--bevel-sunken) win2k:focus:shadow-(--bevel-sunken) win2k:dark:focus:shadow-(--bevel-sunken)",
      className,
    )}
    {...props}
  >
    {children}
    <SelectPrimitive.Icon asChild>
      {/* Win2000: 下拉框右端一枚凸起的箭头按钮 */}
      <ChevronDown className="h-4 w-4 opacity-50 win2k:h-[18px] win2k:w-4 win2k:shrink-0 win2k:p-[3px] win2k:opacity-100 win2k:stroke-3 win2k:bg-(--w-face) win2k:text-(--w-text) win2k:shadow-(--bevel-raised)" />
    </SelectPrimitive.Icon>
  </SelectPrimitive.Trigger>
));
SelectTrigger.displayName = SelectPrimitive.Trigger.displayName;

export const SelectContent = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Content> & {
    /** 固定在滚动列表上方的插槽 (如搜索框), 不随 Viewport 滚动。 */
    header?: React.ReactNode;
  }
>(({ className, children, header, position = "popper", ...props }, ref) => (
  <SelectPrimitive.Portal>
    <SelectPrimitive.Content
      ref={ref}
      className={cn(
        "relative z-50 max-h-96 min-w-32 overflow-hidden bg-(--surface) text-(--ink) data-[state=open]:animate-in data-[state=closed]:animate-out " +
          "plain:rounded-(--r-sm) plain:border plain:border-(--line-2) plain:shadow-md " +
          "sketch:rounded-(--r-card-alt) sketch:border-2 sketch:border-(--stroke) sketch:shadow-[4px_4px_0_var(--fill-oat)] " +
          "win2k:border-(--w-text) win2k:bg-(--w-win) win2k:shadow-none",
        position === "popper" &&
          "data-[side=bottom]:translate-y-1 data-[side=left]:-translate-x-1 data-[side=right]:translate-x-1 data-[side=top]:-translate-y-1",
        className,
      )}
      position={position}
      {...props}
    >
      {header != null && (
        <div className="border-b border-(--line) p-1">{header}</div>
      )}
      <SelectPrimitive.Viewport
        className={cn(
          "p-1 max-h-[384px] overflow-y-auto",
          position === "popper" &&
            "h-(--radix-select-trigger-height) w-full min-w-(--radix-select-trigger-width)",
        )}
      >
        {children}
      </SelectPrimitive.Viewport>
    </SelectPrimitive.Content>
  </SelectPrimitive.Portal>
));
SelectContent.displayName = SelectPrimitive.Content.displayName;

export const SelectItem = React.forwardRef<
  React.ElementRef<typeof SelectPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Item> & {
    /** 下拉项里显示在主标题下方的小字，不会出现在 trigger 里。 */
    subtitle?: React.ReactNode;
  }
>(({ className, children, subtitle, ...props }, ref) => (
  <SelectPrimitive.Item
    ref={ref}
    className={cn(
      "relative flex w-full cursor-default select-none items-start py-1.5 pl-2 pr-8 text-[13px] outline-hidden data-disabled:pointer-events-none data-disabled:opacity-50 " +
        "plain:rounded-sm plain:focus:bg-(--surface-3) sketch:rounded-(--r-sketch) sketch:focus:bg-(--fill-butter) " +
        "win2k:focus:bg-(--w-sel) win2k:focus:text-(--w-selt)",
      className,
    )}
    {...props}
  >
    <span className="absolute right-2 top-2 flex h-3.5 w-3.5 items-center justify-center">
      <SelectPrimitive.ItemIndicator>
        <Check className="h-4 w-4" />
      </SelectPrimitive.ItemIndicator>
    </span>
    <div className="flex flex-col min-w-0 flex-1">
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
      {subtitle != null && (
        <span className="mt-0.5 truncate text-[10px] font-mono text-(--ink-4)">
          {subtitle}
        </span>
      )}
    </div>
  </SelectPrimitive.Item>
));
SelectItem.displayName = SelectPrimitive.Item.displayName;
