import { createFileRoute, Outlet } from "@tanstack/react-router";
import { useAuth } from "@/authContext";
import { Redirect } from "@/components/app/redirect";
import { canViewNotifications } from "@/utils/permissions";

export const Route = createFileRoute("/admin/notifications")({
  component: AdminNotificationsLayout,
});

function AdminNotificationsLayout() {
  const { me } = useAuth();
  // Signed out: the shell redirects to /login, so do not compete with it.
  if (!me.authenticated) return null;
  if (!canViewNotifications(me)) return <Redirect to="/workspace/repositories" replace />;
  return <Outlet />;
}
