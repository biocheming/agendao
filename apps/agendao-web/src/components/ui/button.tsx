import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { Slot } from "radix-ui"

import { cn } from "@/lib/utils"

const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-2 rounded-xl border border-transparent text-sm font-medium tracking-tight whitespace-nowrap transition-[color,background-color,border-color,box-shadow,transform] outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/45 disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default: "bg-foreground text-background shadow-sm hover:bg-foreground/92",
        destructive:
          "bg-destructive text-white shadow-sm hover:bg-destructive/92 focus-visible:ring-destructive/20 dark:bg-destructive/60 dark:focus-visible:ring-destructive/40",
        outline:
          "border-border/60 bg-background/78 text-foreground hover:border-border hover:bg-accent/55 dark:bg-input/22 dark:hover:bg-input/36",
        secondary:
          "bg-secondary/82 text-secondary-foreground hover:bg-secondary",
        ghost:
          "bg-transparent text-muted-foreground hover:bg-accent/55 hover:text-foreground dark:hover:bg-accent/50",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default: "h-10 px-4 py-2 has-[>svg]:px-3.5",
        xs: "h-7 gap-1 rounded-lg px-2.5 text-xs has-[>svg]:px-2 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-9 gap-1.5 rounded-xl px-3.5 has-[>svg]:px-3",
        lg: "h-11 rounded-2xl px-6 has-[>svg]:px-5",
        icon: "size-10 rounded-full",
        "icon-xs": "size-7 rounded-full [&_svg:not([class*='size-'])]:size-3",
        "icon-sm": "size-8 rounded-full",
        "icon-lg": "size-11 rounded-full",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function Button({
  className,
  variant = "default",
  size = "default",
  asChild = false,
  ...props
}: React.ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean
  }) {
  const Comp = asChild ? Slot.Root : "button"

  return (
    <Comp
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  )
}

export { Button, buttonVariants }
