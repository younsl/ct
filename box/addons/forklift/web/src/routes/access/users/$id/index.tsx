import { createFileRoute } from "@tanstack/react-router";

import { useAuth } from "@/authContext";
import { Redirect } from "@/components/app/redirect";
import { UserDetailPage } from "@/routes/access/users/-components/user-detail-page";
import { canViewUsers } from "@/utils/permissions";

export const Route = createFileRoute("/access/users/$id/")({
  component: UserDetailRoute,
});

function UserDetailRoute() {
  const { me } = useAuth();

  // Signed out: the shell redirects to /login, so do not compete with it.
  if (!me.authenticated) return null;

  return canViewUsers(me)
    ? <UserDetailPage me={me} />
    : <Redirect to="/workspace/repositories" replace />;
}
