import { useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, test, vi } from "vitest";

import {
  PermissionPicker,
  type PermissionEntry,
  type PermissionPickerMode,
} from "@/components/app/permission-picker/permission-picker";
import type { PermissionName } from "@/lib/permission-catalog";
import * as repositoriesApi from "@/services/v1/repositories/api";
import { useUserPreferences } from "@/stores/user-preferences";

vi.mock("@/services/v1/repositories/api");

const mockedRepositories = vi.mocked(repositoriesApi);

function Harness({
  mode,
  initial = [],
  grantable,
}: {
  mode: PermissionPickerMode;
  initial?: PermissionEntry[];
  grantable?: (name: PermissionName) => boolean;
}) {
  const [entries, setEntries] = useState<PermissionEntry[]>(initial);
  return (
    <>
      <PermissionPicker mode={mode} value={entries} onChange={setEntries} grantable={grantable} />
      <output data-testid="entries">{JSON.stringify(entries)}</output>
    </>
  );
}

function renderPicker(
  mode: PermissionPickerMode,
  initial?: PermissionEntry[],
  grantable?: (name: PermissionName) => boolean,
) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <Harness mode={mode} initial={initial} grantable={grantable} />
    </QueryClientProvider>,
  );
  return {
    entries: () => JSON.parse(screen.getByTestId("entries").textContent ?? "[]") as PermissionEntry[],
  };
}

const option = (name: string) => screen.getByRole("checkbox", { name });
const patternField = () => screen.queryByLabelText("Repository pattern");

describe("PermissionPicker", () => {
  beforeEach(() => {
    useUserPreferences.setState({ language: "en" });
    mockedRepositories.listRepositoryNames.mockResolvedValue([]);
  });

  test("starts at repo.read on every repository", () => {
    renderPicker("role");

    expect(option("repo.read")).toHaveAttribute("aria-checked", "true");
    expect(patternField()).toHaveValue("*");
  });

  test("hides the repository field when only global permissions are chosen", async () => {
    const user = userEvent.setup();
    const picker = renderPicker("role");

    await user.click(option("repo.read"));
    await user.click(option("user.read"));
    await user.click(option("audit.read"));

    expect(patternField()).not.toBeInTheDocument();
    expect(screen.getByText(/Global permissions apply to the whole instance/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Add permission" }));

    expect(picker.entries()).toEqual([{ repo_pattern: "", actions: ["user.read", "audit.read"] }]);
    expect(screen.getByRole("listitem", { name: "Global: user.read, audit.read" })).toBeInTheDocument();
  });

  test("shows the repository field once a repo.* permission is chosen and splits a mixed selection", async () => {
    const user = userEvent.setup();
    const picker = renderPicker("role");

    await user.click(option("repo.write"));
    await user.click(option("role.read"));

    expect(patternField()).toBeInTheDocument();
    expect(screen.getByText(/added as a separate entry/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Add permission" }));

    expect(picker.entries()).toEqual([
      { repo_pattern: "*", actions: ["repo.read", "repo.write"] },
      { repo_pattern: "", actions: ["role.read"] },
    ]);
    // The selection starts over after an add.
    expect(option("repo.write")).toHaveAttribute("aria-checked", "false");
    expect(option("role.read")).toHaveAttribute("aria-checked", "false");
  });

  test("merges an add into the entry with the same pattern", async () => {
    const user = userEvent.setup();
    const picker = renderPicker("role", [{ repo_pattern: "*", actions: ["repo.read"] }]);

    await user.click(option("repo.delete"));
    await user.click(screen.getByRole("button", { name: "Add permission" }));

    expect(picker.entries()).toEqual([{ repo_pattern: "*", actions: ["repo.read", "repo.delete"] }]);
  });

  test("repo.admin covers the other repo.* permissions and stores only itself", async () => {
    const user = userEvent.setup();
    const picker = renderPicker("role");

    await user.click(option("repo.admin"));

    expect(option("repo.write")).toHaveAttribute("aria-checked", "true");
    expect(option("repo.write")).toHaveAttribute("aria-disabled", "true");
    expect(screen.getAllByText("Included in repo.admin").length).toBeGreaterThan(0);

    await user.click(option("repo.write"));
    await user.click(screen.getByRole("button", { name: "Add permission" }));

    expect(picker.entries()).toEqual([{ repo_pattern: "*", actions: ["repo.admin"] }]);
  });

  test("full access is offered to roles only", async () => {
    const user = userEvent.setup();
    const picker = renderPicker("role");

    await user.click(option("*"));

    expect(option("user.write")).toHaveAttribute("aria-disabled", "true");
    expect(patternField()).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Add permission" }));

    expect(picker.entries()).toEqual([{ repo_pattern: "", actions: ["*"] }]);
  });

  test("token mode offers only what a token may carry", () => {
    renderPicker("token");

    for (const name of ["*", "repo.approve", "repo.security", "repo.admin", "user.write", "system.write"]) {
      expect(screen.queryByRole("checkbox", { name, exact: true })).not.toBeInTheDocument();
    }
    for (const name of ["repo.read", "repo.write", "repo.delete", "user.read", "system.read"]) {
      expect(option(name)).not.toHaveAttribute("aria-disabled");
    }
  });

  test("a role grant beyond the caller's own permissions is locked", async () => {
    const user = userEvent.setup();
    renderPicker("role", [], (name) => ["user.read", "repo.read"].includes(name));

    expect(option("user.read")).not.toHaveAttribute("aria-disabled");
    expect(option("user.write")).toHaveAttribute("aria-disabled", "true");
    expect(option("*")).toHaveAttribute("aria-disabled", "true");

    await user.click(option("user.write"));

    expect(option("user.write")).toHaveAttribute("aria-checked", "false");
  });

  test("removes an entry", async () => {
    const user = userEvent.setup();
    const picker = renderPicker("token", [
      { repo_pattern: "maven-*", actions: ["repo.read"] },
      { repo_pattern: "", actions: ["audit.read"] },
    ]);

    const list = screen.getByRole("list", { name: "Granted permissions" });
    await user.click(
      within(list).getByRole("button", { name: "Remove permission maven-*: repo.read" }),
    );

    expect(picker.entries()).toEqual([{ repo_pattern: "", actions: ["audit.read"] }]);
  });
});
