import { PermissionEntryList } from "@/components/app/permission-picker/permission-entry-list";
import {
  PermissionSelector,
  type PermissionPickerMode,
} from "@/components/app/permission-picker/permission-selector";
import { useTranslation } from "@/lib/i18n";
import type { PermissionName } from "@/lib/permission-catalog";
import {
  mergePermissionEntries,
  removePermissionEntryAt,
  type PermissionEntry,
} from "@/lib/permission-entries";

export type { PermissionEntry, PermissionPickerMode };

// The one permission editor for roles and tokens, holding the entries locally
// until the caller saves them. The role detail page grants against the API one
// add at a time, so it composes PermissionEntryList and PermissionSelector
// itself instead.
export function PermissionPicker({
  mode,
  value,
  onChange,
  grantable,
}: {
  mode: PermissionPickerMode;
  value: PermissionEntry[];
  onChange: (entries: PermissionEntry[]) => void;
  grantable?: (name: PermissionName) => boolean;
}) {
  const { t } = useTranslation();

  return (
    <div className="flex min-w-0 flex-col gap-3">
      <PermissionEntryList
        entries={value}
        emptyText={t("common.no-permissions-added")}
        onRemove={(index) => onChange(removePermissionEntryAt(value, index))}
      />
      <PermissionSelector
        mode={mode}
        grantable={grantable}
        onAdd={(added) => onChange(mergePermissionEntries(value, added))}
      />
    </div>
  );
}
