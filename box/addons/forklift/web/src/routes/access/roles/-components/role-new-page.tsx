import { useState, type FormEvent } from "react";
import { useNavigate } from "@tanstack/react-router";

import { useAuth } from "@/authContext";
import { Alert } from "@/components/app-ui/alert";
import { Badge } from "@/components/app-ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { PageDescription, PageHeader } from "@/components/app-ui/page";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { PermissionPicker, type PermissionEntry } from "@/components/app/permission-picker/permission-picker";
import { NAME_PATTERN } from "@/lib/name-pattern";
import { useTranslation } from "@/lib/i18n";
import { useRoleCreateSubmit } from "@/routes/access/roles/-hooks/use-role-create-submit";
import { hasPermission } from "@/utils/permissions";

// Admin-only role creation, reached from the Create button on /access/roles.
// Permissions can be granted here at creation, or added later on the role's
// detail page.
export function RoleNewPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { me } = useAuth();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [permissions, setPermissions] = useState<PermissionEntry[]>([]);
  const { error, isPending, submit } = useRoleCreateSubmit();

  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    submit({
      name,
      // Both are omitted rather than sent empty: the API treats an absent
      // field as "not given", and an empty list as "grant nothing".
      description: description || undefined,
      permissions: permissions.length ? permissions : undefined,
    });
  };

  return (
    <div data-testid="page-role-new">
      <PageHeader title={t("role.create")} />
      <PageDescription>{t("role.new-description")}</PageDescription>

      <Card size="sm" className="mb-4 max-w-[44rem]">
        <CardContent>
          <form onSubmit={onSubmit} className="space-y-5">
            <FieldGroup className="gap-4">
              <Field>
                <FieldLabel htmlFor="role-name">
                  {t("common.role-name")}<span className="text-destructive">*</span>
                </FieldLabel>
                <Input
                  id="role-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="maven-readers"
                  autoFocus
                  required
                  pattern={NAME_PATTERN}
                />
                <FieldDescription>{t("common.name-rule")}</FieldDescription>
              </Field>

              <Field>
                <FieldLabel htmlFor="role-description">{t("common.description")}</FieldLabel>
                <Input
                  id="role-description"
                  value={description}
                  onChange={(event) => setDescription(event.target.value)}
                  placeholder={t("common.optional")}
                />
              </Field>
            </FieldGroup>

            <section aria-labelledby="role-permissions-heading" className="flex min-w-0 flex-col gap-3 border-t border-border pt-4">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <h2 id="role-permissions-heading" className="m-0 text-sm font-semibold">{t("common.permissions")}</h2>
                  <p className="mt-1 mb-0 text-sm text-muted-foreground">{t("role.permissions-description")}</p>
                </div>
                <Badge className="mt-0.5">{permissions.length}</Badge>
              </div>
              <PermissionPicker
                mode="role"
                value={permissions}
                onChange={setPermissions}
                grantable={(permission) => hasPermission(me, permission)}
              />
            </section>

            {error && <Alert>{error}</Alert>}

            <div className="flex min-w-0 items-center gap-2 border-t border-border pt-4 max-sm:flex-wrap">
              <Button type="submit" disabled={!name.trim() || isPending}>
                {t("role.create")}
              </Button>
              <Button
                variant="outline"
                type="button"
                onClick={() => navigate({ to: "/access/roles" })}
              >
                {t("common.cancel")}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
