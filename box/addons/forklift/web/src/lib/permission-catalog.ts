import type { MessageKey } from "@/lib/i18n";
import type { PermissionName, TokenScope } from "@/services/v1/openapi-types";

export type { PermissionName };

export type TokenPermissionName = TokenScope["actions"][number];

export type PermissionCategory =
  | "repo"
  | "user"
  | "role"
  | "audit"
  | "approval"
  | "notification"
  | "coverage"
  | "insights"
  | "system"
  | "all";

export type PermissionScope = "repo" | "global";

export type PermissionDefinition = {
  name: PermissionName;
  category: PermissionCategory;
  scope: PermissionScope;
  tokenAllowed: boolean;
  descriptionKey: MessageKey;
};

// Keyed by the generated union so a permission added to or dropped from the
// OpenAPI document fails tsc here instead of drifting silently.
const DEFINITIONS = {
  "repo.read": { category: "repo", descriptionKey: "permission.repo.read.description" },
  "repo.write": { category: "repo", descriptionKey: "permission.repo.write.description" },
  "repo.delete": { category: "repo", descriptionKey: "permission.repo.delete.description" },
  "repo.approve": { category: "repo", descriptionKey: "permission.repo.approve.description" },
  "repo.security": { category: "repo", descriptionKey: "permission.repo.security.description" },
  "repo.admin": { category: "repo", descriptionKey: "permission.repo.admin.description" },
  "user.read": { category: "user", descriptionKey: "permission.user.read.description" },
  "user.write": { category: "user", descriptionKey: "permission.user.write.description" },
  "role.read": { category: "role", descriptionKey: "permission.role.read.description" },
  "role.write": { category: "role", descriptionKey: "permission.role.write.description" },
  "audit.read": { category: "audit", descriptionKey: "permission.audit.read.description" },
  "approval.read": { category: "approval", descriptionKey: "permission.approval.read.description" },
  "notification.read": { category: "notification", descriptionKey: "permission.notification.read.description" },
  "notification.write": { category: "notification", descriptionKey: "permission.notification.write.description" },
  "coverage.read": { category: "coverage", descriptionKey: "permission.coverage.read.description" },
  "coverage.write": { category: "coverage", descriptionKey: "permission.coverage.write.description" },
  "insights.read": { category: "insights", descriptionKey: "permission.insights.read.description" },
  "insights.write": { category: "insights", descriptionKey: "permission.insights.write.description" },
  "system.read": { category: "system", descriptionKey: "permission.system.read.description" },
  "system.write": { category: "system", descriptionKey: "permission.system.write.description" },
  "*": { category: "all", descriptionKey: "permission.all.description" },
} as const satisfies Record<PermissionName, { category: PermissionCategory; descriptionKey: MessageKey }>;

// Exactly the TokenScope enum: a name missing here or one the API does not
// accept on a token fails tsc.
const TOKEN_PERMISSIONS = {
  "repo.read": true,
  "repo.write": true,
  "repo.delete": true,
  "user.read": true,
  "role.read": true,
  "audit.read": true,
  "approval.read": true,
  "notification.read": true,
  "coverage.read": true,
  "insights.read": true,
  "system.read": true,
} as const satisfies Record<TokenPermissionName, true>;

function hasOwn(record: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(record, key);
}

export const ALL_PERMISSION: PermissionName = "*";

export const PERMISSIONS: PermissionDefinition[] = (
  Object.keys(DEFINITIONS) as PermissionName[]
).map((name) => ({
  name,
  category: DEFINITIONS[name].category,
  scope: DEFINITIONS[name].category === "repo" ? "repo" : "global",
  tokenAllowed: hasOwn(TOKEN_PERMISSIONS, name),
  descriptionKey: DEFINITIONS[name].descriptionKey,
}));

export const PERMISSION_NAMES: PermissionName[] = PERMISSIONS.map((definition) => definition.name);

export const CATEGORY_LABEL_KEYS = {
  repo: "permission.category.repo",
  user: "permission.category.user",
  role: "permission.category.role",
  audit: "permission.category.audit",
  approval: "permission.category.approval",
  notification: "permission.category.notification",
  coverage: "permission.category.coverage",
  insights: "permission.category.insights",
  system: "permission.category.system",
  all: "permission.category.all",
} as const satisfies Record<PermissionCategory, MessageKey>;

export const GLOBAL_CATEGORIES = [
  "user",
  "role",
  "audit",
  "approval",
  "notification",
  "coverage",
  "insights",
  "system",
] as const satisfies readonly PermissionCategory[];

export function isPermissionName(name: string): name is PermissionName {
  return hasOwn(DEFINITIONS, name);
}

export function permissionDefinition(name: string): PermissionDefinition | undefined {
  return PERMISSIONS.find((definition) => definition.name === name);
}

export function isRepoPermission(name: string): boolean {
  return permissionDefinition(name)?.scope === "repo";
}

export function isGlobalPermission(name: string): boolean {
  return permissionDefinition(name)?.scope === "global";
}

export function isTokenPermission(name: string): name is TokenPermissionName {
  return hasOwn(TOKEN_PERMISSIONS, name);
}

export function permissionsInCategory(category: PermissionCategory): PermissionDefinition[] {
  return PERMISSIONS.filter((definition) => definition.category === category);
}

// Catalog order, so chips and stored lists read the same everywhere.
export function sortPermissions<T extends string>(names: readonly T[]): T[] {
  const rank = (name: string) => {
    const index = PERMISSION_NAMES.indexOf(name as PermissionName);
    return index === -1 ? PERMISSION_NAMES.length : index;
  };
  return [...names].sort((left, right) => rank(left) - rank(right));
}
