import { ALL_PERMISSION, type PermissionName } from "@/lib/permission-catalog";
import type { Me, RepositoryCapabilities } from "@/services/v1/openapi-types";

// Only the permission fields matter here, so a partial principal is enough - the
// callers hold a full Me, but the rules do not depend on the rest of it. admin
// is kept as a fallback for a server that predates the permissions list.
type Principal = Pick<Me, "admin" | "permissions">;

type Capabilities = Pick<RepositoryCapabilities, "admin" | "security">;

export function hasPermission(principal: Principal, name: PermissionName): boolean {
  if (principal.admin) return true;
  const held = principal.permissions ?? [];
  return held.includes(ALL_PERMISSION) || held.includes(name);
}

export function hasAnyPermission(principal: Principal, names: readonly PermissionName[]): boolean {
  return names.some((name) => hasPermission(principal, name));
}

export function hasFullAccess(principal: Principal): boolean {
  return hasPermission(principal, ALL_PERMISSION);
}

export function canViewUsers(principal: Principal): boolean {
  return hasPermission(principal, "user.read");
}

export function canManageUsers(principal: Principal): boolean {
  return hasPermission(principal, "user.write");
}

export function canViewRoles(principal: Principal): boolean {
  return hasPermission(principal, "role.read");
}

export function canManageRoles(principal: Principal): boolean {
  return hasPermission(principal, "role.write");
}

export function canViewAudit(principal: Principal): boolean {
  return hasPermission(principal, "audit.read");
}

export function canViewApprovalQueue(principal: Principal): boolean {
  return hasAnyPermission(principal, ["approval.read", "repo.approve"]);
}

export function canReviewApprovals(principal: Principal): boolean {
  return hasPermission(principal, "repo.approve");
}

export function canViewNotifications(principal: Principal): boolean {
  return hasPermission(principal, "notification.read");
}

export function canManageNotifications(principal: Principal): boolean {
  return hasPermission(principal, "notification.write");
}

export function canViewCoverageSettings(principal: Principal): boolean {
  return hasPermission(principal, "coverage.read");
}

export function canManageCoverage(principal: Principal): boolean {
  return hasPermission(principal, "coverage.write");
}

export function canViewSystem(principal: Principal): boolean {
  return hasPermission(principal, "system.read");
}

export function canManageSystem(principal: Principal): boolean {
  return hasPermission(principal, "system.write");
}

// repo.admin held on any pattern: the server checks the new name against it.
export function canCreateRepositories(principal: Principal): boolean {
  return hasPermission(principal, "repo.admin");
}

// The policy columns and tab are readable by auditors and by whoever may edit
// the policy of at least one repository.
export function canViewRepositoryPolicy(principal: Principal): boolean {
  return hasAnyPermission(principal, ["audit.read", "repo.security", "repo.admin"]);
}

// Per repository, from its capabilities. A repository read from a server that
// predates them falls back to the instance-wide superuser.
export function canEditRepositorySecurity(principal: Principal, capabilities?: Capabilities): boolean {
  return capabilities ? capabilities.security : hasFullAccess(principal);
}

export function canAdministerRepository(principal: Principal, capabilities?: Capabilities): boolean {
  return capabilities ? capabilities.admin : hasFullAccess(principal);
}
