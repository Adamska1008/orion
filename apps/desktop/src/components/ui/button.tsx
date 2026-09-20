import * as React from 'react';
import { Slot } from '@radix-ui/react-slot';
import { cva, type VariantProps } from 'class-variance-authority';
import { cn } from '@/lib/utils';

// shadcn/ui-style owned component: Radix Slot + cva, customized for desktop density.
const buttonVariants = cva('inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-lg text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-40 [&_svg]:size-4 shrink-0', {
  variants: {
    variant: { default: 'bg-primary text-primary-foreground hover:bg-primary/90', outline: 'border border-border bg-surface text-foreground hover:bg-muted', ghost: 'hover:bg-muted text-muted-foreground', destructive: 'bg-destructive-surface text-destructive-foreground hover:bg-destructive-hover' },
    size: { default: 'h-9 px-3.5', sm: 'h-8 px-2.5 text-xs', icon: 'size-9' },
  },
  defaultVariants: { variant: 'default', size: 'default' },
});

type Props = React.ComponentProps<'button'> & VariantProps<typeof buttonVariants> & { asChild?: boolean };
export function Button({ className, variant, size, asChild = false, ...props }: Props) {
  const Component = asChild ? Slot : 'button';
  return <Component className={cn(buttonVariants({ variant, size, className }))} {...props} />;
}
