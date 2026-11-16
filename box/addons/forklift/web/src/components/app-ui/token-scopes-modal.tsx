import { useState } from "react";

import { Alert } from "@/components/app-ui/alert";
import { HintTooltip } from "@/components/app-ui/hint-tooltip";
import { PermissionPicker, type PermissionEntry } from "@/components/app/permission-picker/permission-picker";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { getErrorMessage } from "@/lib/http/error/api-error";
import { useTranslation } from "@/lib/i18n";
import { toPermissionEntries, toTokenScopes } from "@/lib/permission-entries";
import { parseTokenScopes, type TokenScope } from "@/utils/token-scopes";

import type { Token } from "@/services/v1/openapi-types";

// TokenScopesModal edits the permissions of an existing token in place: scopes
// can be added and removed without re-issuing the secret. Scopes only ever
// narrow the owner's role permissions (enforced server-side at auth time), so
// widening the list here cannot grant access the owner does not already have.
export function TokenScopesModal({ token, onSave, onClose }: {
  token: Token;
  // Persists the full replacement scope list (self-service or admin endpoint).
  onSave: (scopes: TokenScope[]) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<PermissionEntry[]>(() =>
    toPermissionEntries(parseTokenScopes(token.scopes_json)),
  );
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const scopes = toTokenScopes(entries);

  const save = async () => {
    setError("");
    setBusy(true);
    try {
      await onSave(scopes);
      onClose();
    } catch (err) {
      setError(getErrorMessage(err));
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{t("token.edit-permissions")}</DialogTitle>
          <DialogDescription>
            <span className="mb-1 block font-mono text-foreground">{token.name}</span>
            {t("token.edit-permissions-note")}
          </DialogDescription>
        </DialogHeader>

        <PermissionPicker mode="token" value={entries} onChange={setEntries} />

        {error && <Alert>{error}</Alert>}
        <DialogFooter>
          <Button variant="outline" type="button" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <HintTooltip content={scopes.length === 0 ? t("token.scopes-required") : undefined} wrap>
            <Button type="button" disabled={busy || scopes.length === 0} onClick={save}>
              {busy ? t("common.saving") : t("common.save")}
            </Button>
          </HintTooltip>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
