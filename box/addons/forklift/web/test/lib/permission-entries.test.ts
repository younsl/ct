import { describe, expect, test } from "vitest";

import {
  PERMISSIONS,
  PERMISSION_NAMES,
  isRepoPermission,
  isTokenPermission,
  permissionDefinition,
  sortPermissions,
} from "@/lib/permission-catalog";
import {
  canAddSelection,
  formatPermissionEntry,
  impliedBy,
  mergePermissionEntries,
  normalizeSelection,
  removePermissionEntryAt,
  splitSelection,
  toPermissionEntries,
  toTokenScopes,
  toggleSelection,
  withoutHeld,
} from "@/lib/permission-entries";

import en from "@/locales/en.json";
import ko from "@/locales/ko.json";

describe("permission catalog", () => {
  test("lists every permission once, repo.* first and * last", () => {
    expect(new Set(PERMISSION_NAMES).size).toBe(PERMISSION_NAMES.length);
    expect(PERMISSION_NAMES[0]).toBe("repo.read");
    expect(PERMISSION_NAMES[PERMISSION_NAMES.length - 1]).toBe("*");
  });

  test("only repo.* permissions are scoped to repositories", () => {
    for (const definition of PERMISSIONS) {
      expect(definition.scope).toBe(definition.name.startsWith("repo.") ? "repo" : "global");
    }
    expect(permissionDefinition("*")?.scope).toBe("global");
  });

  test("tokens carry reads and repository writes, never decisions or global writes", () => {
    const tokenable = PERMISSIONS.filter((definition) => definition.tokenAllowed).map((d) => d.name);
    expect(tokenable).toEqual([
      "repo.read",
      "repo.write",
      "repo.delete",
      "user.read",
      "role.read",
      "audit.read",
      "approval.read",
      "notification.read",
      "coverage.read",
      "insights.read",
      "system.read",
    ]);
    expect(isTokenPermission("repo.approve")).toBe(false);
    expect(isTokenPermission("*")).toBe(false);
    expect(isTokenPermission("toString")).toBe(false);
  });

  test("every description has copy in both languages", () => {
    for (const definition of PERMISSIONS) {
      expect(en[definition.descriptionKey]).toBeTruthy();
      expect(ko[definition.descriptionKey]).toBeTruthy();
    }
  });

  test("sorting follows the catalog and keeps unknown names last", () => {
    expect(sortPermissions(["user.read", "legacy", "repo.write", "repo.read"])).toEqual([
      "repo.read",
      "repo.write",
      "user.read",
      "legacy",
    ]);
  });
});

describe("selection", () => {
  test("* implies everything else and repo.admin every other repo.* permission", () => {
    expect(impliedBy(["*"], "user.read")).toBe("*");
    expect(impliedBy(["*"], "*")).toBeNull();
    expect(impliedBy(["repo.admin"], "repo.write")).toBe("repo.admin");
    expect(impliedBy(["repo.admin"], "user.read")).toBeNull();
    expect(impliedBy(["repo.admin"], "repo.admin")).toBeNull();
  });

  test("an implying permission keeps the stored selection minimal", () => {
    expect(normalizeSelection(["repo.read", "repo.write", "repo.admin", "user.read"])).toEqual([
      "repo.admin",
      "user.read",
    ]);
    expect(normalizeSelection(["repo.read", "user.write", "*"])).toEqual(["*"]);
  });

  test("toggling adds, removes, and leaves implied permissions alone", () => {
    expect(toggleSelection(["repo.read"], "repo.write")).toEqual(["repo.read", "repo.write"]);
    expect(toggleSelection(["repo.read", "repo.write"], "repo.write")).toEqual(["repo.read"]);
    expect(toggleSelection(["repo.read"], "repo.admin")).toEqual(["repo.admin"]);
    expect(toggleSelection(["repo.admin"], "repo.read")).toEqual(["repo.admin"]);
  });

  test("a repository permission needs a pattern, a global one does not", () => {
    expect(canAddSelection([], "*")).toBe(false);
    expect(canAddSelection(["repo.read"], "  ")).toBe(false);
    expect(canAddSelection(["repo.read"], "maven-*")).toBe(true);
    expect(canAddSelection(["user.read"], "")).toBe(true);
    expect(canAddSelection(["*"], "")).toBe(true);
  });

  test("a mixed selection splits into one entry per kind", () => {
    expect(splitSelection(["user.read", "repo.read", "audit.read"], " maven-* ")).toEqual([
      { repo_pattern: "maven-*", actions: ["repo.read"] },
      { repo_pattern: "", actions: ["user.read", "audit.read"] },
    ]);
    expect(splitSelection(["*"], "*")).toEqual([{ repo_pattern: "", actions: ["*"] }]);
    expect(splitSelection(["repo.write"], "npm-*")).toEqual([
      { repo_pattern: "npm-*", actions: ["repo.write"] },
    ]);
  });
});

describe("entries", () => {
  test("merging folds into an entry with the same pattern and kind", () => {
    const existing = [
      { repo_pattern: "maven-*", actions: ["repo.read" as const] },
      { repo_pattern: "", actions: ["user.read" as const] },
    ];
    const merged = mergePermissionEntries(existing, [
      { repo_pattern: "maven-*", actions: ["repo.write", "repo.read"] },
      { repo_pattern: "", actions: ["audit.read"] },
      { repo_pattern: "npm-*", actions: ["repo.read"] },
    ]);

    expect(merged).toEqual([
      { repo_pattern: "maven-*", actions: ["repo.read", "repo.write"] },
      { repo_pattern: "", actions: ["user.read", "audit.read"] },
      { repo_pattern: "npm-*", actions: ["repo.read"] },
    ]);
    expect(existing[0].actions).toEqual(["repo.read"]);
  });

  test("merging repo.admin into an entry collapses what it implies", () => {
    expect(
      mergePermissionEntries(
        [{ repo_pattern: "*", actions: ["repo.read", "repo.write"] }],
        [{ repo_pattern: "*", actions: ["repo.admin"] }],
      ),
    ).toEqual([{ repo_pattern: "*", actions: ["repo.admin"] }]);
  });

  test("withoutHeld drops what a stored entry of the same pattern grants", () => {
    const stored = [
      { repo_pattern: "maven-*", actions: ["repo.admin"] },
      { repo_pattern: "", actions: ["user.read"] },
    ];
    expect(
      withoutHeld(stored, [
        { repo_pattern: "maven-*", actions: ["repo.write"] },
        { repo_pattern: "", actions: ["user.read", "role.read"] },
      ]),
    ).toEqual([{ repo_pattern: "", actions: ["role.read"] }]);
  });

  test("removing by index keeps duplicate patterns apart", () => {
    const entries = [
      { repo_pattern: "maven-*", actions: ["repo.read" as const] },
      { repo_pattern: "maven-*", actions: ["repo.write" as const] },
    ];
    expect(removePermissionEntryAt(entries, 0)).toEqual([entries[1]]);
  });

  test("a repository entry reads as pattern, a global one as the Global label", () => {
    expect(formatPermissionEntry({ repo_pattern: "maven-*", actions: ["repo.write", "repo.read"] }, "Global"))
      .toBe("maven-*: repo.read, repo.write");
    expect(formatPermissionEntry({ repo_pattern: "", actions: ["audit.read", "user.read"] }, "Global"))
      .toBe("Global: user.read, audit.read");
    expect(formatPermissionEntry({ actions: ["system.read"] }, "전역")).toBe("전역: system.read");
  });

  test("token scopes keep only what a token may carry and omit an empty pattern", () => {
    expect(
      toTokenScopes([
        { repo_pattern: "maven-*", actions: ["repo.read", "repo.approve"] },
        { repo_pattern: "", actions: ["user.read", "user.write"] },
        { repo_pattern: "", actions: ["*"] },
      ]),
    ).toEqual([
      { repo_pattern: "maven-*", actions: ["repo.read"] },
      { actions: ["user.read"] },
    ]);
  });

  test("stored entries without a pattern read as global", () => {
    expect(toPermissionEntries([{ actions: ["user.read"] }, { repo_pattern: "a", actions: ["repo.read", "read"] }]))
      .toEqual([
        { repo_pattern: "", actions: ["user.read"] },
        { repo_pattern: "a", actions: ["repo.read"] },
      ]);
    expect(isRepoPermission("read")).toBe(false);
  });
});
