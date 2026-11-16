import { describe, expect, test } from "vitest";

import {
  formatTokenScope,
  formatTokenScopes,
  parseTokenScopes,
} from "@/utils/token-scopes";

// scopes_json is a JSON string on the wire, so anything can be in it - a token
// written before the feature existed, or by an older server. A malformed value
// means "no scopes recorded", never a broken page.
describe("parseTokenScopes", () => {
  test("reads a scope list", () => {
    expect(parseTokenScopes('[{"repo_pattern":"maven-*","actions":["repo.read"]},{"actions":["audit.read"]}]')).toEqual([
      { repo_pattern: "maven-*", actions: ["repo.read"] },
      { actions: ["audit.read"] },
    ]);
  });

  test("an empty, malformed or non-array value reads as no scopes", () => {
    expect(parseTokenScopes("")).toEqual([]);
    expect(parseTokenScopes("not json")).toEqual([]);
    expect(parseTokenScopes("null")).toEqual([]);
    expect(parseTokenScopes('{"repo_pattern":"maven-*"}')).toEqual([]);
  });
});

describe("formatting", () => {
  test("a repository scope reads as pattern then permissions", () => {
    expect(formatTokenScope({ repo_pattern: "maven-*", actions: ["repo.read", "repo.write"] }, "Global"))
      .toBe("maven-*: repo.read, repo.write");
  });

  test("a scope without a pattern reads as global", () => {
    expect(formatTokenScope({ repo_pattern: "", actions: ["audit.read"] }, "Global")).toBe("Global: audit.read");
    expect(formatTokenScope({ actions: ["user.read"] }, "Global")).toBe("Global: user.read");
  });

  test("a whole list joins for sorting", () => {
    expect(
      formatTokenScopes('[{"repo_pattern":"a","actions":["repo.read"]},{"actions":["system.read"]}]', "Global"),
    ).toBe("a: repo.read, Global: system.read");
  });
});
