import { describe, expect, it } from "vitest";

import { formatMilliseconds } from "@/utils/format-duration";
import { formatFileSize } from "@/utils/format-file-size";
import {
  canAdministerRepository,
  canCreateRepositories,
  canEditRepositorySecurity,
  canManageRoles,
  canManageSystem,
  canReviewApprovals,
  canViewApprovalQueue,
  canViewRepositoryPolicy,
  canViewRoles,
  canViewUsers,
  hasPermission,
} from "@/utils/permissions";
import { getRepositoryEndpoint } from "@/utils/repository-endpoint";

describe("formatFileSize", () => {
  it("stays in bytes below a kilobyte", () => {
    expect(formatFileSize(0)).toBe("0 B");
    expect(formatFileSize(1023)).toBe("1023 B");
  });

  it("climbs one unit at a time", () => {
    expect(formatFileSize(1024)).toBe("1.0 KB");
    expect(formatFileSize(1024 ** 3)).toBe("1.0 GB");
  });

  it("stops at the largest unit it knows", () => {
    expect(formatFileSize(1024 ** 5)).toBe("1024.0 TB");
  });
});

describe("formatMilliseconds", () => {
  it("switches unit at one second", () => {
    expect(formatMilliseconds(999)).toBe("999ms");
    expect(formatMilliseconds(1000)).toBe("1.0s");
  });
});

describe("getRepositoryEndpoint", () => {
  const origin = "https://forklift.example";

  it("gives each ecosystem the shape its tooling expects", () => {
    expect(getRepositoryEndpoint("cargo", "crates", origin).url).toBe(
      "sparse+https://forklift.example/cargo/crates/",
    );
    // go is the one without a trailing slash
    expect(getRepositoryEndpoint("go", "goproxy", origin).url).toBe(
      "https://forklift.example/go/goproxy",
    );
    expect(getRepositoryEndpoint("pypi", "pypi", origin).url).toBe(
      "https://forklift.example/pypi/pypi/simple/",
    );
  });

  it("falls back to a plain path for an unknown format", () => {
    const endpoint = getRepositoryEndpoint("raw", "files", origin);
    expect(endpoint.url).toBe("https://forklift.example/raw/files/");
    expect(endpoint.hint).toBe("");
  });
});

describe("permissions", () => {
  it("holds exactly the permissions the server listed", () => {
    const me = { permissions: ["user.read" as const, "repo.read" as const] };
    expect(hasPermission(me, "user.read")).toBe(true);
    expect(hasPermission(me, "user.write")).toBe(false);
    expect(hasPermission({}, "repo.read")).toBe(false);
  });

  it("treats * as every permission", () => {
    const me = { permissions: ["*" as const] };
    expect(canViewUsers(me)).toBe(true);
    expect(canManageRoles(me)).toBe(true);
    expect(canManageSystem(me)).toBe(true);
    expect(canCreateRepositories(me)).toBe(true);
  });

  it("falls back to the admin flag when the list is missing", () => {
    expect(canManageSystem({ admin: true })).toBe(true);
    expect(canManageSystem({ admin: false })).toBe(false);
  });

  it("lets an auditor read the access surfaces but not the approval decision", () => {
    const auditor = { permissions: ["audit.read" as const, "user.read" as const, "role.read" as const, "approval.read" as const] };
    expect(canViewUsers(auditor)).toBe(true);
    expect(canViewRoles(auditor)).toBe(true);
    expect(canManageRoles(auditor)).toBe(false);
    expect(canViewApprovalQueue(auditor)).toBe(true);
    expect(canReviewApprovals(auditor)).toBe(false);
  });

  it("opens the approval queue to a reviewer without approval.read", () => {
    const approver = { permissions: ["repo.approve" as const] };
    expect(canViewApprovalQueue(approver)).toBe(true);
    expect(canReviewApprovals(approver)).toBe(true);
  });

  it("reads repository edits from the repository's own capabilities", () => {
    const security = { permissions: ["repo.security" as const] };
    expect(canViewRepositoryPolicy(security)).toBe(true);
    expect(canEditRepositorySecurity(security, { security: true, admin: false })).toBe(true);
    expect(canEditRepositorySecurity(security, { security: false, admin: false })).toBe(false);
    expect(canAdministerRepository(security, { security: true, admin: false })).toBe(false);
    expect(canViewUsers(security)).toBe(false);
  });

  it("without capabilities only * edits a repository", () => {
    expect(canAdministerRepository({ permissions: ["repo.admin"] })).toBe(false);
    expect(canAdministerRepository({ permissions: ["*"] })).toBe(true);
  });
});
