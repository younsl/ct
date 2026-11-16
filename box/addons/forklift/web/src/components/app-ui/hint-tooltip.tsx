import type { ReactElement, ReactNode } from "react";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

// Puts a styled tooltip on one element, or renders the element alone when there
// is nothing to say. A disabled control receives no pointer or focus events, so
// `wrap` hangs the tooltip on a focusable span around it instead.
export function HintTooltip({
  content,
  children,
  side = "top",
  wrap = false,
}: {
  content: ReactNode;
  children: ReactElement;
  side?: "top" | "right" | "bottom" | "left";
  wrap?: boolean;
}) {
  if (!content) return children;

  return (
    <Tooltip>
      {wrap ? (
        <TooltipTrigger render={<span tabIndex={0} className="inline-flex max-w-full *:flex-1" />}>
          {children}
        </TooltipTrigger>
      ) : (
        <TooltipTrigger render={children} />
      )}
      <TooltipContent side={side}>{content}</TooltipContent>
    </Tooltip>
  );
}
