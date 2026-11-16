import { useId, useState, type ReactNode } from "react";
import { Boxes, Globe, Plus, ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import { RepositoryPatternCombobox } from "@/components/app/repository-pattern-combobox";
import {
  PermissionOption,
  type PermissionOptionState,
} from "@/components/app/permission-picker/permission-option";
import {
  ALL_REPOSITORIES_PATTERN,
  useRepositoryPatternOptions,
} from "@/hooks/repositories/use-repository-pattern-options";
import { useTranslation } from "@/lib/i18n";
import {
  CATEGORY_LABEL_KEYS,
  GLOBAL_CATEGORIES,
  isGlobalPermission,
  isRepoPermission,
  permissionsInCategory,
  type PermissionDefinition,
  type PermissionName,
} from "@/lib/permission-catalog";
import {
  canAddSelection,
  impliedBy,
  splitSelection,
  toggleSelection,
  type PermissionEntry,
} from "@/lib/permission-entries";
import { cn } from "@/lib/utils";

export type PermissionPickerMode = "role" | "token";

const DEFAULT_SELECTION: PermissionName[] = ["repo.read"];

// Step 1 picks permissions, step 2 (only when a repo.* permission is picked)
// the repository pattern. Add hands back at most two entries, one per kind,
// and starts the selection over.
export function PermissionSelector({
  mode,
  onAdd,
  onCancel,
  isAdding = false,
  grantable,
}: {
  mode: PermissionPickerMode;
  onAdd: (entries: PermissionEntry[]) => void;
  onCancel?: () => void;
  isAdding?: boolean;
  // Permissions the caller may hand out. The server refuses a grant beyond the
  // caller's own reach, so those are shown locked rather than failing on save.
  grantable?: (name: PermissionName) => boolean;
}) {
  const { t } = useTranslation();
  const stepOneId = useId();
  const stepTwoId = useId();
  const patternId = useId();
  const [selected, setSelected] = useState<PermissionName[]>(() =>
    DEFAULT_SELECTION.filter((name) => !grantable || grantable(name)),
  );
  const [pattern, setPattern] = useState(ALL_REPOSITORIES_PATTERN);
  const { options: repoOptions, types: repoTypes } = useRepositoryPatternOptions();

  const hasRepo = selected.some(isRepoPermission);
  const hasGlobal = selected.some(isGlobalPermission);
  const canAdd = canAddSelection(selected, pattern) && !isAdding;

  // A token never carries session-only permissions, so token mode leaves them
  // out instead of showing cards that can never be picked.
  const offered = (category: Parameters<typeof permissionsInCategory>[0]) =>
    permissionsInCategory(category).filter((definition) => mode !== "token" || definition.tokenAllowed);

  const isLocked = (definition: PermissionDefinition) => (grantable ? !grantable(definition.name) : false);

  const optionState = (definition: PermissionDefinition): PermissionOptionState => {
    if (isLocked(definition)) return "unavailable";
    if (impliedBy(selected, definition.name)) return "implied";
    return selected.includes(definition.name) ? "on" : "off";
  };

  const optionHint = (definition: PermissionDefinition): string | undefined => {
    if (grantable && !grantable(definition.name)) return t("permission.not-held");
    const by = impliedBy(selected, definition.name);
    return by ? t("permission.implied-by").replace("{name}", by) : undefined;
  };

  const renderOption = (definition: PermissionDefinition) => (
    <PermissionOption
      key={definition.name}
      definition={definition}
      state={optionState(definition)}
      hint={optionHint(definition)}
      onToggle={() => setSelected((current) => toggleSelection(current, definition.name))}
    />
  );

  const add = () => {
    if (!canAdd) return;
    onAdd(splitSelection(selected, pattern));
    setSelected(DEFAULT_SELECTION.filter((name) => !grantable || grantable(name)));
    setPattern(ALL_REPOSITORIES_PATTERN);
  };

  return (
    <div className="flex min-w-0 flex-col gap-4 rounded-lg border border-border/80 bg-background/40 p-3 sm:p-4">
      <section aria-labelledby={stepOneId} className="flex min-w-0 flex-col gap-3">
        <StepHeading id={stepOneId} step={1}>
          {t("permission.step-what")}
        </StepHeading>
        {mode === "token" && (
          <p className="m-0 text-xs leading-relaxed text-muted-foreground">{t("permission.token-note")}</p>
        )}

        <ScopeGroup
          icon={<Boxes className="size-3.5" aria-hidden="true" />}
          title={t(CATEGORY_LABEL_KEYS.repo)}
          hint={t("permission.scope-repo")}
        >
          <div role="group" aria-label={t(CATEGORY_LABEL_KEYS.repo)} className="grid min-w-0 gap-2 sm:grid-cols-2">
            {offered("repo").map(renderOption)}
          </div>
        </ScopeGroup>

        <ScopeGroup
          icon={<Globe className="size-3.5" aria-hidden="true" />}
          title={t("permission.group-instance")}
          hint={t("permission.scope-global")}
        >
          {mode === "token" ? (
            <div role="group" aria-label={t("permission.group-instance")} className="grid min-w-0 gap-2 sm:grid-cols-2">
              {GLOBAL_CATEGORIES.flatMap((category) => offered(category)).map(renderOption)}
            </div>
          ) : (
          <div className="flex min-w-0 flex-col gap-3">
            {GLOBAL_CATEGORIES.filter((category) => offered(category).length > 0).map((category) => (
              <div key={category} role="group" aria-label={t(CATEGORY_LABEL_KEYS[category])} className="flex min-w-0 flex-col gap-1.5">
                <span className="text-xs font-medium text-muted-foreground">{t(CATEGORY_LABEL_KEYS[category])}</span>
                <div className="grid min-w-0 gap-2 sm:grid-cols-2">{offered(category).map(renderOption)}</div>
              </div>
            ))}
          </div>
          )}
        </ScopeGroup>

        {mode === "role" && (
          <ScopeGroup
            icon={<ShieldAlert className="size-3.5" aria-hidden="true" />}
            title={t(CATEGORY_LABEL_KEYS.all)}
            hint={t("permission.scope-all")}
          >
            <div role="group" aria-label={t(CATEGORY_LABEL_KEYS.all)} className="grid min-w-0 gap-2 sm:grid-cols-2">
              {offered("all").map(renderOption)}
            </div>
          </ScopeGroup>
        )}
      </section>

      {hasRepo ? (
        <section aria-labelledby={stepTwoId} className="flex min-w-0 flex-col gap-3 border-t border-border pt-4">
          <StepHeading id={stepTwoId} step={2}>
            {t("permission.step-where")}
          </StepHeading>
          <Field>
            <FieldLabel htmlFor={patternId}>{t("common.repository-pattern")}</FieldLabel>
            <RepositoryPatternCombobox
              id={patternId}
              className="w-full"
              options={repoOptions}
              types={repoTypes}
              value={pattern}
              onValueChange={setPattern}
            />
            <FieldDescription>{t("permission.pattern-hint")}</FieldDescription>
          </Field>
          {hasGlobal && <p className="m-0 text-xs leading-relaxed text-muted-foreground">{t("permission.mixed-note")}</p>}
        </section>
      ) : (
        <p
          className={cn(
            "m-0 border-t border-border pt-3 text-xs leading-relaxed text-muted-foreground",
            !hasGlobal && "sr-only",
          )}
          aria-live="polite"
        >
          {hasGlobal ? t("permission.global-note") : t("permission.select-hint")}
        </p>
      )}

      <div className="flex min-w-0 flex-wrap items-center justify-end gap-2">
        {onCancel && (
          <Button variant="ghost" type="button" onClick={onCancel}>
            {t("common.cancel")}
          </Button>
        )}
        <Button variant="outline" type="button" onClick={add} disabled={!canAdd}>
          <Plus data-icon="inline-start" />
          {t("common.add-permission")}
        </Button>
      </div>
    </div>
  );
}

function StepHeading({ id, step, children }: { id: string; step: number; children: ReactNode }) {
  return (
    <h3 id={id} className="m-0 flex items-center gap-2 text-sm font-semibold">
      <span
        aria-hidden="true"
        className="flex size-5 shrink-0 items-center justify-center rounded-full border border-border bg-muted text-[11px] tabular-nums text-muted-foreground"
      >
        {step}
      </span>
      {children}
    </h3>
  );
}

function ScopeGroup({
  icon,
  title,
  hint,
  children,
}: {
  icon: ReactNode;
  title: string;
  hint: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
        <span className="inline-flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-foreground">
          {icon}
          {title}
        </span>
        <span className="text-xs text-muted-foreground">{hint}</span>
      </div>
      {children}
    </div>
  );
}
