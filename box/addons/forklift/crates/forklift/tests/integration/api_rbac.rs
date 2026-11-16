use http::{Method, StatusCode};

use forklift::meta::{ManagedGrant, ManagedRBAC, ManagedRole, Permission};

use forklift::testing::api::new_test_server;

#[tokio::test]
async fn managed_rbac_read_only_via_api() {
    let srv = new_test_server().await;

    let desired = ManagedRBAC {
        roles: vec![ManagedRole {
            name: "readonly".to_string(),
            permissions: vec![Permission {
                repo_pattern: "*".to_string(),
                actions: "repo.read".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        group_roles: vec![ManagedGrant {
            subject: "/devs".to_string(),
            role: "readonly".to_string(),
        }],
        user_roles: vec![ManagedGrant {
            subject: "alice".to_string(),
            role: "readonly".to_string(),
        }],
        ..Default::default()
    };
    srv.store
        .apply_managed_rbac(desired)
        .await
        .expect("seed managed rbac");

    // GET /roles exposes the managed flag and the permission id.
    let roles = srv.admin_do(Method::GET, "/roles", "").await.json();
    let readonly = roles
        .as_array()
        .expect("roles")
        .iter()
        .find(|r| r["name"] == "readonly")
        .unwrap_or_else(|| panic!("readonly role not reported: {roles}"))
        .clone();
    let role_id = readonly["id"].as_i64().expect("role id");
    assert_ne!(role_id, 0, "{roles}");
    assert_eq!(readonly["managed"], true, "{roles}");
    let perms = readonly["permissions"].as_array().expect("permissions");
    assert_eq!(perms.len(), 1, "{readonly}");
    let perm_id = perms[0]["id"].as_i64().expect("permission id");

    // Deleting a managed role is rejected.
    let resp = srv
        .admin_do(Method::DELETE, &format!("/roles/{role_id}"), "")
        .await;
    assert_eq!(resp.status, StatusCode::CONFLICT, "delete managed role");

    // Adding a permission to a managed role is rejected.
    let resp = srv
        .admin_do(
            Method::POST,
            &format!("/roles/{role_id}/permissions"),
            r#"{"repo_pattern":"*","actions":["repo.write"]}"#,
        )
        .await;
    assert_eq!(
        resp.status,
        StatusCode::CONFLICT,
        "add perm to managed role"
    );

    // Deleting a managed permission is rejected.
    let resp = srv
        .admin_do(
            Method::DELETE,
            &format!("/roles/{role_id}/permissions/{perm_id}"),
            "",
        )
        .await;
    assert_eq!(resp.status, StatusCode::CONFLICT, "delete managed perm");

    // Deleting a managed group mapping is rejected.
    let mappings = srv
        .store
        .list_group_mappings()
        .await
        .expect("list group mappings");
    let resp = srv
        .admin_do(
            Method::DELETE,
            &format!("/group-mappings/{}", mappings[0].id),
            "",
        )
        .await;
    assert_eq!(resp.status, StatusCode::CONFLICT, "delete managed mapping");

    // Removing a managed user-role assignment is rejected.
    let alice = srv
        .store
        .get_user_by_username("alice")
        .await
        .expect("alice");
    let resp = srv
        .admin_do(
            Method::DELETE,
            &format!("/users/{}/roles/{role_id}", alice.id),
            "",
        )
        .await;
    assert_eq!(
        resp.status,
        StatusCode::CONFLICT,
        "remove managed user role"
    );
}

#[tokio::test]
async fn unmanaged_role_still_mutable() {
    let srv = new_test_server().await;

    // A role created via the API is unmanaged and fully mutable.
    let resp = srv
        .admin_do(
            Method::POST,
            "/roles",
            r#"{"name":"manual","description":"x","permissions":[{"repo_pattern":"*","actions":["repo.read"]}]}"#,
        )
        .await;
    assert_eq!(resp.status, StatusCode::CREATED, "create role");
    let role = resp.json();
    assert_eq!(
        role["managed"], false,
        "an API-created role must be unmanaged"
    );
    let id = role["id"].as_i64().expect("role id");
    let resp = srv
        .admin_do(Method::DELETE, &format!("/roles/{id}"), "")
        .await;
    assert_eq!(resp.status, StatusCode::NO_CONTENT, "delete unmanaged role");
}

/// `repo.admin` is bounded by its pattern: it manages, creates and groups only
/// repositories whose names match, and opens none of the instance surfaces.
#[tokio::test]
async fn repo_admin_is_bounded_by_its_pattern() {
    let srv = new_test_server().await;
    forklift::testing::api::mk_role_user(
        &srv,
        "lead",
        "pw123456",
        "team-admin",
        "team-*",
        r#""repo.admin""#,
    )
    .await;
    let as_lead = async |method: Method, uri: &str, body: &str| {
        srv.do_as("lead", "pw123456", method, uri, body).await
    };

    let resp = as_lead(
        Method::POST,
        "/repositories",
        r#"{"name":"team-maven","format":"maven","type":"hosted"}"#,
    )
    .await;
    assert_eq!(resp.status, StatusCode::CREATED, "{}", resp.text());
    let team_id = resp.json()["id"].as_i64().expect("repo id");
    let resp = as_lead(
        Method::POST,
        "/repositories",
        r#"{"name":"other-maven","format":"maven","type":"hosted"}"#,
    )
    .await;
    assert_eq!(
        resp.status,
        StatusCode::FORBIDDEN,
        "create outside the pattern"
    );

    let resp = srv
        .admin_do(
            Method::POST,
            "/repositories",
            r#"{"name":"secret-maven","format":"maven","type":"hosted"}"#,
        )
        .await;
    assert_eq!(resp.status, StatusCode::CREATED, "{}", resp.text());
    let secret_id = resp.json()["id"].as_i64().expect("repo id");

    let resp = as_lead(
        Method::POST,
        &format!("/repositories/{team_id}/disabled"),
        r#"{"disabled":true}"#,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK, "{}", resp.text());
    let resp = as_lead(
        Method::POST,
        &format!("/repositories/{secret_id}/disabled"),
        r#"{"disabled":true}"#,
    )
    .await;
    assert_eq!(
        resp.status,
        StatusCode::FORBIDDEN,
        "disable outside the pattern"
    );
    let resp = as_lead(Method::DELETE, &format!("/repositories/{secret_id}"), "").await;
    assert_eq!(
        resp.status,
        StatusCode::FORBIDDEN,
        "delete outside the pattern"
    );

    let resp = as_lead(
        Method::POST,
        "/repositories",
        r#"{"name":"team-group","format":"maven","type":"group","config":{"group":{"members":["team-maven","secret-maven"]}}}"#,
    )
    .await;
    assert_eq!(
        resp.status,
        StatusCode::FORBIDDEN,
        "a group must not expose a repository outside the pattern: {}",
        resp.text()
    );

    for uri in ["/users", "/roles", "/ha", "/storage"] {
        let resp = as_lead(Method::GET, uri, "").await;
        assert_eq!(resp.status, StatusCode::FORBIDDEN, "GET {uri}");
    }
    let me = as_lead(Method::GET, "/me", "").await.json();
    assert_eq!(me["admin"], false, "{me}");
    let perms: Vec<&str> = me["permissions"]
        .as_array()
        .expect("permissions")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        perms.contains(&"repo.admin")
            && perms.contains(&"repo.delete")
            && !perms.contains(&"user.read"),
        "{me}"
    );
}

/// `role.write` and `user.write` without `*` must not turn into more: a
/// delegated manager grants only what it holds and cannot take over an account
/// holding more.
#[tokio::test]
async fn delegated_managers_cannot_escalate() {
    let srv = new_test_server().await;
    forklift::testing::api::mk_role_user(
        &srv,
        "mgr",
        "pw123456",
        "access-manager",
        "*",
        r#""repo.read","role.write","user.write""#,
    )
    .await;
    let as_mgr = async |method: Method, uri: &str, body: &str| {
        srv.do_as("mgr", "pw123456", method, uri, body).await.status
    };

    for (what, body) in [
        (
            "superuser",
            r#"{"name":"r1","permissions":[{"actions":["*"]}]}"#,
        ),
        (
            "repo.admin",
            r#"{"name":"r2","permissions":[{"repo_pattern":"*","actions":["repo.admin"]}]}"#,
        ),
        (
            "unheld global",
            r#"{"name":"r3","permissions":[{"actions":["system.write"]}]}"#,
        ),
    ] {
        assert_eq!(
            as_mgr(Method::POST, "/roles", body).await,
            StatusCode::FORBIDDEN,
            "{what}"
        );
    }
    assert_eq!(
        as_mgr(
            Method::POST,
            "/roles",
            r#"{"name":"readers","permissions":[{"repo_pattern":"maven-*","actions":["repo.read"]}]}"#,
        )
        .await,
        StatusCode::CREATED,
        "a grant within reach"
    );

    let users = srv.admin_do(Method::GET, "/users", "").await.json();
    let id_of = |name: &str| {
        users
            .as_array()
            .expect("users")
            .iter()
            .find(|u| u["username"] == name)
            .and_then(|u| u["id"].as_i64())
            .expect("user id")
    };
    let (admin_id, mgr_id) = (id_of("admin"), id_of("mgr"));
    let roles = srv.admin_do(Method::GET, "/roles", "").await.json();
    let administrator = roles
        .as_array()
        .expect("roles")
        .iter()
        .find(|r| r["name"] == "administrator")
        .and_then(|r| r["id"].as_i64())
        .expect("administrator role");

    assert_eq!(
        as_mgr(
            Method::POST,
            &format!("/users/{mgr_id}/roles"),
            &format!(r#"{{"role_id":{administrator}}}"#),
        )
        .await,
        StatusCode::FORBIDDEN,
        "assign administrator to self"
    );
    assert_eq!(
        as_mgr(
            Method::POST,
            "/group-mappings",
            &format!(r#"{{"group_name":"/anyone","role_id":{administrator}}}"#),
        )
        .await,
        StatusCode::FORBIDDEN,
        "map a group to administrator"
    );
    assert_eq!(
        as_mgr(
            Method::PUT,
            &format!("/users/{admin_id}"),
            r#"{"password":"taken-over-1"}"#,
        )
        .await,
        StatusCode::FORBIDDEN,
        "reset the administrator's password"
    );
    assert_eq!(
        as_mgr(
            Method::POST,
            &format!("/users/{admin_id}/tokens"),
            r#"{"name":"x","description":"x","scopes":[{"repo_pattern":"*","actions":["repo.read"]}],"expires_in":"1h"}"#,
        )
        .await,
        StatusCode::FORBIDDEN,
        "issue a token for the administrator"
    );
    assert_eq!(
        as_mgr(Method::DELETE, &format!("/users/{admin_id}"), "").await,
        StatusCode::FORBIDDEN,
        "delete the administrator"
    );
}

/// A delegated manager cannot take access away from someone holding more:
/// removing an administrator's role or deleting a role beyond its reach is
/// refused like granting one.
#[tokio::test]
async fn delegated_managers_cannot_strip_wider_access() {
    let srv = new_test_server().await;
    forklift::testing::api::mk_role_user(
        &srv,
        "mgr",
        "pw123456",
        "access-manager",
        "*",
        r#""repo.read","role.write","user.write""#,
    )
    .await;
    let users = srv.admin_do(Method::GET, "/users", "").await.json();
    let admin_id = users
        .as_array()
        .expect("users")
        .iter()
        .find(|u| u["username"] == "admin")
        .and_then(|u| u["id"].as_i64())
        .expect("admin id");
    let roles = srv.admin_do(Method::GET, "/roles", "").await.json();
    let administrator = roles
        .as_array()
        .expect("roles")
        .iter()
        .find(|r| r["name"] == "administrator")
        .and_then(|r| r["id"].as_i64())
        .expect("administrator role");
    let as_mgr = async |method: Method, uri: &str| {
        srv.do_as("mgr", "pw123456", method, uri, "").await.status
    };
    assert_eq!(
        as_mgr(
            Method::DELETE,
            &format!("/users/{admin_id}/roles/{administrator}")
        )
        .await,
        StatusCode::FORBIDDEN,
        "strip the administrator's role"
    );
    assert_eq!(
        as_mgr(Method::DELETE, &format!("/roles/{administrator}")).await,
        StatusCode::FORBIDDEN,
        "delete the administrator role"
    );
}
