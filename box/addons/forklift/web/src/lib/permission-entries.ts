import {
  ALL_PERMISSION,
  isGlobalPermission,
  isPermissionName,
  isRepoPermission,
  isTokenPermission,
  sortPermissions,
  type PermissionName,
} from "@/lib/permission-catalog";
import type { TokenScope } from "@/services/v1/openapi-types";

// One role permission or token scope as the editors hold it. repo_pattern is
// "" for global permissions and a repository glob for repo.* ones, and an entry
// never mixes the two kinds (the API answers 400 if it does).
export type PermissionEntry = {
  repo_pattern: string;
  actions: PermissionName[];
};

// What a stored entry may hold: a token or role written before the catalog can
// carry names the editors no longer offer, which must still display.
export type StoredPermissionEntry = {
  repo_pattern?: string;
  actions: readonly string[];
};

const REPO_ADMIN: PermissionName = "repo.admin";

// The permission that already grants `name` within the selection, if any:
// * grants everything, and repo.admin grants every other repo.* permission.
export function impliedBy(selected: readonly PermissionName[], name: PermissionName): PermissionName | null {
  if (name !== ALL_PERMISSION && selected.includes(ALL_PERMISSION)) return ALL_PERMISSION;
  if (name !== REPO_ADMIN && isRepoPermission(name) && selected.includes(REPO_ADMIN)) return REPO_ADMIN;
  return null;
}

// Keeps a selection minimal: what an implying permission already grants is
// dropped, so only * or repo.admin is stored rather than everything under it.
export function normalizeSelection(selected: readonly PermissionName[]): PermissionName[] {
  const unique = [...new Set(selected)];
  return sortPermissions(unique.filter((name) => impliedBy(unique, name) === null));
}

export function toggleSelection(selected: readonly PermissionName[], name: PermissionName): PermissionName[] {
  if (impliedBy(selected, name)) return [...selected];
  return selected.includes(name)
    ? selected.filter((current) => current !== name)
    : normalizeSelection([...selected, name]);
}

export function canAddSelection(selected: readonly PermissionName[], pattern: string): boolean {
  if (selected.length === 0) return false;
  return !selected.some(isRepoPermission) || pattern.trim() !== "";
}

// Splits a selection into at most two entries, one per kind, in the order the
// API wants them: the repository entry carries the pattern, the global one none.
export function splitSelection(selected: readonly PermissionName[], pattern: string): PermissionEntry[] {
  const normalized = normalizeSelection(selected);
  const repo = normalized.filter(isRepoPermission);
  const global = normalized.filter(isGlobalPermission);
  const entries: PermissionEntry[] = [];
  if (repo.length > 0 && pattern.trim()) entries.push({ repo_pattern: pattern.trim(), actions: repo });
  if (global.length > 0) entries.push({ repo_pattern: "", actions: global });
  return entries;
}

function entryKind(entry: StoredPermissionEntry): "repo" | "global" | null {
  if (entry.actions.length === 0 || !entry.actions.every(isPermissionName)) return null;
  if (entry.actions.every(isRepoPermission)) return entry.repo_pattern ? "repo" : null;
  if (entry.actions.every(isGlobalPermission)) return entry.repo_pattern ? null : "global";
  return null;
}

// Folds new entries into a list: an entry with the same pattern and kind gains
// the new permissions instead of a second row. Anything the catalog does not
// recognise is left as it is and the new entry is appended beside it.
export function mergePermissionEntries(
  existing: readonly PermissionEntry[],
  added: readonly PermissionEntry[],
): PermissionEntry[] {
  const merged = existing.map((entry) => ({ ...entry, actions: [...entry.actions] }));
  for (const entry of added) {
    const kind = entryKind(entry);
    const target = kind
      ? merged.find((candidate) => candidate.repo_pattern === entry.repo_pattern && entryKind(candidate) === kind)
      : undefined;
    if (target) {
      target.actions = normalizeSelection([...target.actions, ...entry.actions]);
    } else {
      merged.push({ repo_pattern: entry.repo_pattern, actions: normalizeSelection(entry.actions) });
    }
  }
  return merged;
}

// The part of `added` not already held by an existing entry of the same
// pattern, for editors that grant one entry at a time against the API.
export function withoutHeld(
  existing: readonly StoredPermissionEntry[],
  added: readonly PermissionEntry[],
): PermissionEntry[] {
  return added
    .map((entry) => {
      const held = existing
        .filter((candidate) => (candidate.repo_pattern ?? "") === entry.repo_pattern)
        .flatMap((candidate) => candidate.actions);
      return {
        ...entry,
        actions: entry.actions.filter(
          (name) => !held.includes(name) && !(isRepoPermission(name) && held.includes(REPO_ADMIN)),
        ),
      };
    })
    .filter((entry) => entry.actions.length > 0);
}

export function removePermissionEntryAt<T>(entries: readonly T[], index: number): T[] {
  return entries.filter((_, current) => current !== index);
}

export function isGlobalEntry(entry: StoredPermissionEntry): boolean {
  return !entry.repo_pattern;
}

// "maven-*: repo.read, repo.write" or "Global: user.read". globalLabel is the
// translated word for an entry without a pattern.
export function formatPermissionEntry(entry: StoredPermissionEntry, globalLabel: string): string {
  const scope = entry.repo_pattern ? entry.repo_pattern : globalLabel;
  return `${scope}: ${sortPermissions(entry.actions).join(", ")}`;
}

// Stored entries in the shape the editors take. Unknown action names are
// dropped, which is what saving through a catalog-only editor does anyway.
export function toPermissionEntries(entries: readonly StoredPermissionEntry[]): PermissionEntry[] {
  return entries.map((entry) => ({
    repo_pattern: entry.repo_pattern ?? "",
    actions: entry.actions.filter(isPermissionName),
  }));
}

export function toTokenScopes(entries: readonly PermissionEntry[]): TokenScope[] {
  return entries
    .map((entry) => ({ ...entry, actions: entry.actions.filter(isTokenPermission) }))
    .filter((entry) => entry.actions.length > 0)
    .map((entry) =>
      entry.repo_pattern ? { repo_pattern: entry.repo_pattern, actions: entry.actions } : { actions: entry.actions },
    );
}
