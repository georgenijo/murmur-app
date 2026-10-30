"use client";

import { Menu } from "@base-ui/react/menu";
import {
  AnimatePresence,
  LayoutGroup,
  motion,
  useReducedMotion,
} from "motion/react";
import {
  createContext,
  type ReactNode,
  useContext,
  useId,
  useState,
} from "react";
import { motionTransition } from "@/lib/sona-motion";
import { cn } from "@/lib/sona-utils";

// ─── Context ─────────────────────────────────────────────────────────────────

interface DropdownContextValue {
  /** Unique layoutId prefix for the hover highlight — scoped per Root instance. */
  layoutId: string;
  /** Currently highlighted item id (for keyboard + mouse parity). */
  activeId: string | null;
  setActiveId: (id: string | null) => void;
}

const DropdownContext = createContext<DropdownContextValue | null>(null);

function useDropdownContext() {
  const ctx = useContext(DropdownContext);
  if (!ctx)
    throw new Error(
      "AnimatedDropdown subcomponents must be used within <AnimatedDropdown>",
    );
  return ctx;
}

// ─── Types ────────────────────────────────────────────────────────────────────

export interface AnimatedDropdownProps {
  children: ReactNode;
  /** Controlled open state. */
  open?: boolean;
  /** Initial open state for uncontrolled usage. @default false */
  defaultOpen?: boolean;
  /** Callback when open state changes. */
  onOpenChange?: (open: boolean) => void;
  /** Whether the menu ignores user interaction. @default false */
  disabled?: boolean;
  /** Whether the open menu limits interaction to the menu. @default true */
  modal?: boolean;
}

export interface AnimatedDropdownContentProps {
  children: ReactNode;
  className?: string;
  /**
   * Side the menu opens on.
   * @default "bottom"
   */
  side?: "bottom" | "top" | "left" | "right";
  /**
   * Alignment along the side.
   * @default "center"
   */
  align?: "start" | "center" | "end";
  /**
   * Gap between trigger and popup in px.
   * @default 6
   */
  sideOffset?: number;
}

export interface AnimatedDropdownItemProps {
  children: ReactNode;
  /** Accessible name when the visible label is not unique. */
  "aria-label"?: string;
  /** Icon to display before the label (any ReactNode — no HugeIcons dependency). */
  icon?: ReactNode;
  /**
   * Visual variant.
   * @default "default"
   */
  variant?: "default" | "danger";
  disabled?: boolean;
  onClick?: () => void;
  className?: string;
}

export interface AnimatedDropdownTriggerProps {
  children: ReactNode;
  className?: string;
  /** Accessible name for icon-only triggers. */
  "aria-label"?: string;
  /** Associates the trigger with an external visible label. */
  "aria-labelledby"?: string;
}

// ─── Root ─────────────────────────────────────────────────────────────────────

/**
 * Root dropdown — owns open state and the hover-highlight layout group.
 * Wrap all other AnimatedDropdown subcomponents inside this.
 */
export function AnimatedDropdown({
  children,
  open,
  defaultOpen = false,
  onOpenChange,
  disabled = false,
  modal = true,
}: AnimatedDropdownProps) {
  const layoutId = useId();
  const [activeId, setActiveId] = useState<string | null>(null);

  const handleOpenChange = (nextOpen: boolean) => {
    if (!nextOpen) setActiveId(null);
    onOpenChange?.(nextOpen);
  };

  return (
    <DropdownContext.Provider value={{ layoutId, activeId, setActiveId }}>
      <LayoutGroup id={layoutId}>
        <Menu.Root
          open={open}
          defaultOpen={defaultOpen}
          disabled={disabled}
          modal={modal}
          onOpenChange={handleOpenChange}
        >
          {children}
        </Menu.Root>
      </LayoutGroup>
    </DropdownContext.Provider>
  );
}

// ─── Trigger ──────────────────────────────────────────────────────────────────

/**
 * The element that opens the dropdown when clicked.
 * Renders as a `<button>` by default via Base UI.
 */
export function AnimatedDropdownTrigger({
  children,
  className,
  "aria-label": ariaLabel,
  "aria-labelledby": ariaLabelledBy,
}: AnimatedDropdownTriggerProps) {
  return (
    <Menu.Trigger
      aria-label={ariaLabel}
      aria-labelledby={ariaLabelledBy}
      className={cn(
        "group inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5",
        "bg-secondary text-secondary-foreground text-sm font-medium",
        "hover:cursor-pointer hover:bg-popover transition-colors duration-150",
        "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
        "data-[popup-open]:bg-popover",
        className,
      )}
    >
      {children}
    </Menu.Trigger>
  );
}

// ─── Content ──────────────────────────────────────────────────────────────────

/**
 * The animated popup panel containing menu items.
 * Scales in from its trigger origin using Base UI's `--transform-origin` variable.
 */
export function AnimatedDropdownContent({
  children,
  className,
  side = "bottom",
  align = "center",
  sideOffset = 6,
}: AnimatedDropdownContentProps) {
  const shouldReduceMotion = useReducedMotion();

  return (
    <Menu.Portal>
      <Menu.Positioner
        side={side}
        align={align}
        sideOffset={sideOffset}
        className="z-50"
      >
        <Menu.Popup
          data-slot="menu-popup"
          className={cn(
            // Glass surface, 8px radius, p-1, scroll within --available-height,
            // scale/fade from Base UI's --transform-origin (styles/menus.css).
            "ui-menu-popup z-50",
            shouldReduceMotion && "duration-0",
            className,
          )}
        >
          {children}
        </Menu.Popup>
      </Menu.Positioner>
    </Menu.Portal>
  );
}

// ─── Item ─────────────────────────────────────────────────────────────────────

/**
 * A single menu item with an animated hover-highlight background.
 * The highlight uses a shared `layoutId` so it glides between items on mouse-over.
 */
export function AnimatedDropdownItem({
  children,
  "aria-label": ariaLabel,
  icon,
  variant = "default",
  disabled,
  onClick,
  className,
}: AnimatedDropdownItemProps) {
  const { layoutId, activeId, setActiveId } = useDropdownContext();
  const itemId = useId();
  const shouldReduceMotion = useReducedMotion();

  const isActive = activeId === itemId;

  return (
    <Menu.Item
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={onClick}
      data-slot="menu-item"
      data-variant={variant === "danger" ? "destructive" : undefined}
      className={cn(
        // 28px row, px-2 py-1 gap-2, 4px radius, 14/20 text (styles/menus.css).
        // The shared motion highlight below paints the hover state instead.
        "ui-menu-item group data-[highlighted]:bg-transparent",
        className,
      )}
      onMouseEnter={() => setActiveId(itemId)}
      onFocus={() => setActiveId(itemId)}
      onMouseLeave={() => setActiveId(null)}
      onBlur={() => setActiveId(null)}
    >
      {/* Animated highlight — shared across all items in this dropdown instance */}
      <AnimatePresence>
        {isActive && (
          <motion.span
            layoutId={shouldReduceMotion ? undefined : `${layoutId}-highlight`}
            className={cn(
              "absolute inset-0 rounded-(--ui-radius-menu-item)",
              variant === "danger"
                ? "bg-[color-mix(in_srgb,var(--murmur-error)_10%,transparent)]"
                : "bg-(--ui-menu-highlight)",
            )}
            initial={shouldReduceMotion ? false : { opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={
              shouldReduceMotion
                ? motionTransition.reduced
                : motionTransition.spatial
            }
          />
        )}
      </AnimatePresence>

      {/* Icon */}
      {icon && (
        <span className="ui-menu-item-icon relative z-10">{icon}</span>
      )}

      {/* Label */}
      <span className="relative z-10 flex min-w-0 flex-1 items-center gap-2">{children}</span>
    </Menu.Item>
  );
}

// ─── Separator ────────────────────────────────────────────────────────────────

/** A thin visual divider between groups of items. */
export function AnimatedDropdownSeparator({
  className,
}: {
  className?: string;
}) {
  return (
    <Menu.Separator className={cn("ui-menu-separator", className)} />
  );
}
