import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  // 外形按画风分套 (plain: / sketch: / win2k: 前缀, 见 styles.css 的 @custom-variant):
  // 经典是 shadcn 原样; 手绘与 styles.css 的 .btn 同款 —— 墨线 + 不规则圆角, 悬停歪一下并落出硬投影;
  // Win2000 继承经典的类, 再用 win2k: 盖成凸起按钮 (立体边框 token 见 themes/win2k.css)
  "inline-flex items-center justify-center gap-2 whitespace-nowrap text-sm font-medium disabled:pointer-events-none disabled:opacity-50 " +
    "plain:rounded-md plain:transition-colors plain:focus-visible:outline-hidden plain:focus-visible:ring-1 plain:focus-visible:ring-ring " +
    "sketch:rounded-(--r-sketch) sketch:border-[1.8px] sketch:border-(--stroke) sketch:transition-[transform,box-shadow,background-color] sketch:duration-150 sketch:hover:-translate-x-px sketch:hover:-translate-y-px sketch:hover:-rotate-[0.6deg] sketch:hover:shadow-[3px_3px_0_var(--fill-oat)] sketch:active:translate-x-px sketch:active:translate-y-px sketch:active:rotate-0 sketch:active:shadow-none sketch:focus-visible:outline-2 sketch:focus-visible:outline-offset-2 sketch:focus-visible:outline-(--accent) sketch:motion-reduce:hover:transform-none " +
    "win2k:bg-(--w-face) win2k:text-(--w-text) win2k:shadow-(--bevel-raised) win2k:hover:bg-(--w-face) win2k:active:shadow-(--bevel-pressed) win2k:focus-visible:ring-0 win2k:focus-visible:outline-1 win2k:focus-visible:outline-dotted win2k:focus-visible:-outline-offset-4 win2k:focus-visible:outline-(--w-text)",
  {
    variants: {
      variant: {
        default:
          "plain:bg-primary plain:text-primary-foreground plain:hover:bg-primary/90 " +
          "sketch:bg-(--accent) sketch:font-semibold sketch:text-[#141413] sketch:hover:shadow-[3px_3px_0_var(--stroke)]",
        destructive:
          "plain:bg-destructive plain:text-destructive-foreground plain:hover:bg-destructive/90 " +
          "sketch:rounded-(--r-sketch-alt) sketch:bg-(--fill-coral) sketch:font-semibold sketch:text-(--ink) sketch:hover:shadow-[3px_3px_0_var(--err)] " +
          "win2k:text-(--err)",
        outline:
          "plain:border plain:border-input plain:bg-background plain:hover:bg-accent plain:hover:text-accent-foreground " +
          "sketch:bg-(--surface) sketch:text-(--ink) win2k:border-0",
        secondary:
          "plain:bg-secondary plain:text-secondary-foreground plain:hover:bg-secondary/80 " +
          "sketch:bg-(--surface-3) sketch:text-(--ink)",
        ghost:
          "plain:hover:bg-accent plain:hover:text-accent-foreground " +
          "sketch:border-transparent sketch:text-(--ink-2) sketch:hover:bg-(--surface-3) sketch:hover:text-(--ink) sketch:hover:shadow-none sketch:hover:transform-none " +
          "win2k:bg-transparent win2k:shadow-none win2k:hover:bg-transparent win2k:hover:shadow-(--bevel-thin) win2k:active:shadow-(--bevel-shallow)",
        link:
          "plain:text-primary plain:underline-offset-4 plain:hover:underline " +
          "sketch:border-transparent sketch:text-(--accent-ink) sketch:underline-offset-4 sketch:hover:underline sketch:hover:shadow-none sketch:hover:transform-none " +
          "win2k:bg-transparent win2k:text-(--accent-ink) win2k:shadow-none win2k:hover:bg-transparent win2k:active:shadow-none",
      },
      size: {
        default: "h-9 px-4 py-2",
        sm: "h-8 px-3 text-xs",
        lg: "h-10 px-6",
        icon: "h-9 w-9",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        className={cn(buttonVariants({ variant, size, className }))}
        ref={ref}
        {...props}
      />
    );
  },
);
Button.displayName = "Button";

export { buttonVariants };
