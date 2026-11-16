import { Globe, X } from "lucide-react";

import { Badge } from "@/components/app-ui/badge";
import { HintTooltip } from "@/components/app-ui/hint-tooltip";
import { Button } from "@/components/ui/button";
import { useTranslation } from "@/lib/i18n";
import { sortPermissions } from "@/lib/permission-catalog";
import { formatPermissionEntry, type StoredPermissionEntry } from "@/lib/permission-entries";

// The entries a role or token holds: a repository entry leads with its pattern,
// a global one with the Global label, then the permissions as chips. Without
// onRemove the list is read-only.
export function PermissionEntryList({
  entries,
  emptyText,
  onRemove,
  getKey = (entry, index) => `${entry.repo_pattern ?? ""}-${index}`,
}: {
  entries: readonly StoredPermissionEntry[];
  emptyText: string;
  onRemove?: (index: number) => void;
  getKey?: (entry: StoredPermissionEntry, index: number) => string | number;
}) {
  const { t } = useTranslation();

  return (
    <ul
      aria-label={t("permission.added")}
      className="m-0 flex min-h-10 min-w-0 list-none flex-col gap-1 rounded-lg border border-border bg-muted/20 p-1.5"
    >
      {entries.map((entry, index) => {
        const summary = formatPermissionEntry(entry, t("permission.global"));
        return (
          <li
            key={getKey(entry, index)}
            aria-label={summary}
            className="flex min-w-0 items-start gap-2 rounded-md px-1.5 py-1 max-sm:flex-wrap"
          >
            <span className="flex min-h-6 min-w-0 shrink-0 items-center sm:w-36">
              {entry.repo_pattern ? (
                <code className="min-w-0 truncate font-mono text-[13px] text-foreground">{entry.repo_pattern}</code>
              ) : (
                <span className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground">
                  <Globe className="size-3.5" aria-hidden="true" />
                  {t("permission.global")}
                </span>
              )}
            </span>
            <span className="flex min-h-6 min-w-0 flex-1 flex-wrap items-center gap-1">
              {sortPermissions(entry.actions).map((action) => (
                <Badge key={action} className="font-mono">
                  {action}
                </Badge>
              ))}
            </span>
            {onRemove && (
              <HintTooltip content={t("common.remove-permission")}>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  className="shrink-0 text-muted-foreground hover:text-foreground"
                  aria-label={`${t("common.remove-permission")} ${summary}`}
                  onClick={() => onRemove(index)}
                >
                  <X className="size-3.5" aria-hidden="true" />
                </Button>
              </HintTooltip>
            )}
          </li>
        );
      })}
      {entries.length === 0 && (
        <li className="flex min-h-7 items-center px-1.5 text-sm text-muted-foreground">{emptyText}</li>
      )}
    </ul>
  );
}
