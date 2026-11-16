import { useState } from "react";
import { Plus } from "lucide-react";

import { useAuth } from "@/authContext";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { PermissionEntryList } from "@/components/app/permission-picker/permission-entry-list";
import { PermissionSelector } from "@/components/app/permission-picker/permission-selector";
import { useTranslation } from "@/lib/i18n";
import { withoutHeld, type PermissionEntry } from "@/lib/permission-entries";
import {
  useAddRolePermissionMutation,
  useRemoveRolePermissionMutation,
} from "@/routes/access/roles/-hooks/use-role-mutations";
import { hasPermission } from "@/utils/permissions";

import type { Role } from "@/services/v1/openapi-types";

export function RolePermissionsPanel({
  role,
  canWrite,
  runAction,
}: {
  role: Role;
  canWrite: boolean;
  runAction: (action: Promise<unknown>) => void;
}) {
  const { t } = useTranslation();
  const { me } = useAuth();
  const [isAdding, setIsAdding] = useState(false);
  const addPermissionMutation = useAddRolePermissionMutation();
  const removePermissionMutation = useRemoveRolePermissionMutation();

  const grant = (entries: PermissionEntry[]) => {
    const missing = withoutHeld(role.permissions, entries);
    // Granted one entry per request, in order, so a failure leaves the earlier
    // ones in place and the page alert names the one that failed.
    runAction(
      missing.reduce<Promise<unknown>>(
        (previous, permission) =>
          previous.then(() => addPermissionMutation.mutateAsync({ roleId: role.id, permission })),
        Promise.resolve(),
      ),
    );
    setIsAdding(false);
  };

  return (
    <Card size="sm" className="mb-4" data-testid="panel-permissions">
      <CardContent className="flex min-w-0 flex-col gap-4">
        <div className="flex min-w-0 items-center justify-between gap-3">
          <h2 className="m-0 text-base font-semibold">{t("common.permissions")}</h2>
          {canWrite && !isAdding && (
            <Button variant="outline" type="button" onClick={() => setIsAdding(true)}>
              <Plus data-icon="inline-start" />
              {t("common.add-permission")}
            </Button>
          )}
        </div>
        <PermissionEntryList
          entries={role.permissions}
          emptyText={t("common.no-permissions-granted")}
          getKey={(_, index) => role.permissions[index].id}
          onRemove={
            canWrite
              ? (index) =>
                  runAction(
                    removePermissionMutation.mutateAsync({
                      roleId: role.id,
                      permissionId: role.permissions[index].id,
                    }),
                  )
              : undefined
          }
        />
        {canWrite && isAdding && (
          <PermissionSelector
            mode="role"
            grantable={(permission) => hasPermission(me, permission)}
            isAdding={addPermissionMutation.isPending}
            onAdd={grant}
            onCancel={() => setIsAdding(false)}
          />
        )}
      </CardContent>
    </Card>
  );
}
