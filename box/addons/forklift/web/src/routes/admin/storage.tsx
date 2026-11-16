import { createFileRoute, Outlet } from "@tanstack/react-router";

import { useAuth } from "@/authContext";
import { Redirect } from "@/components/app/redirect";
import { canViewSystem } from "@/utils/permissions";

export const Route = createFileRoute("/admin/storage")({
  component: AdminStorageLayout,
});

function AdminStorageLayout() {
  const { me } = useAuth();

  // Signed out: the shell redirects to /login, so do not compete with it.
  if (!me.authenticated) return null;

  return canViewSystem(me)
    ? <Outlet />
    : <Redirect to="/workspace/repositories" replace />;
}
