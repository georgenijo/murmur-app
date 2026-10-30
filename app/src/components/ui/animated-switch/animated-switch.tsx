"use client";

import { Switch } from "@base-ui/react/switch";
import { cn } from "@/lib/sona-utils";

export interface AnimatedSwitchProps
  extends Omit<Switch.Root.Props, "checked" | "defaultChecked" | "onCheckedChange" | "className"> {
  checked?: boolean;
  defaultChecked?: boolean;
  onCheckedChange?: (checked: boolean) => void;
  disabled?: boolean;
  size?: "sm" | "md" | "lg";
  className?: string;
}

/** Base UI owns interaction and checked state; CSS owns the shared geometry
 * and motion, including the reduced-motion contract. */
export default function AnimatedSwitch({
  checked, defaultChecked = false, onCheckedChange, disabled = false,
  size = "md", className, ...props
}: AnimatedSwitchProps) {
  return (
    <Switch.Root
      {...props}
      checked={checked}
      {...(checked === undefined ? { defaultChecked } : {})}
      disabled={disabled}
      aria-label={props["aria-label"] ?? (props["aria-labelledby"] ? undefined : "Toggle")}
      onCheckedChange={next => onCheckedChange?.(next)}
      data-size={size}
      className={cn("ui-switch relative inline-flex shrink-0 cursor-pointer items-center focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring data-[disabled]:cursor-not-allowed", className)}
    >
      <Switch.Thumb className="ui-switch-thumb" />
    </Switch.Root>
  );
}
