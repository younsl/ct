import { useId } from "react";
import { Check, Lock } from "lucide-react";

import { HintTooltip } from "@/components/app-ui/hint-tooltip";
import { useTranslation } from "@/lib/i18n";
import { cn } from "@/lib/utils";

import type { PermissionDefinition } from "@/lib/permission-catalog";

export type PermissionOptionState = "off" | "on" | "implied" | "unavailable";

// One permission as a toggle card. Implied and unavailable cards stay focusable
// (aria-disabled, not disabled) so keyboard and pointer users can still reach
// the tooltip that explains why they cannot be changed.
export function PermissionOption({
  definition,
  state,
  hint,
  onToggle,
}: {
  definition: PermissionDefinition;
  state: PermissionOptionState;
  hint?: string;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const descriptionId = useId();
  const checked = state === "on" || state === "implied";
  const locked = state === "implied" || state === "unavailable";

  return (
    <HintTooltip content={state === "unavailable" ? hint : undefined}>
      <button
        type="button"
        role="checkbox"
        aria-checked={checked}
        aria-disabled={locked || undefined}
        aria-label={definition.name}
        aria-describedby={descriptionId}
        data-state={state}
        onClick={() => {
          if (!locked) onToggle();
        }}
        className={cn(
          "flex h-full w-full min-w-0 items-start gap-2.5 rounded-lg border px-3 py-2.5 text-left text-sm transition-colors outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50",
          state === "on" && "border-accent-ink bg-primary/10",
          state === "off" && "border-border bg-muted hover:bg-[var(--fx-surface-3)]",
          state === "implied" && "cursor-default border-border bg-muted",
          state === "unavailable" && "cursor-not-allowed border-dashed border-border bg-transparent",
        )}
      >
        <span
          aria-hidden="true"
          className={cn(
            "mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-[4px] border",
            state === "on" && "border-accent-ink bg-primary text-primary-foreground",
            state === "off" && "border-input",
            state === "implied" && "border-input text-accent-ink",
            state === "unavailable" && "border-transparent text-muted-foreground",
          )}
        >
          {checked && <Check className="size-3.5" />}
          {state === "unavailable" && <Lock className="size-3.5" />}
        </span>
        <span className="min-w-0 flex-1">
          <span
            className={cn(
              "block break-all font-mono text-[13px] font-medium",
              state === "on" && "text-accent-ink",
              state === "unavailable" && "text-muted-foreground",
            )}
          >
            {definition.name}
          </span>
          <span id={descriptionId} className="mt-0.5 block text-xs leading-relaxed text-muted-foreground">
            {t(definition.descriptionKey)}
          </span>
          {state === "implied" && hint && (
            <span className="mt-1 block text-xs font-medium text-foreground">{hint}</span>
          )}
        </span>
      </button>
    </HintTooltip>
  );
}
