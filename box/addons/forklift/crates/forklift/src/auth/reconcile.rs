//! Applies the declarative RBAC policy (ConfigMap) and local accounts (Secret)
//! to the metadata store on startup.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;

use super::authz::{Scope, is_legacy, translate_legacy, translate_legacy_scopes};
use super::credentials::hash_password;
use super::policy::parse_policy;
use crate::meta;

/// Rewrites role permissions and token scopes stored in the pre-catalog
/// vocabulary (`read`, `admin`, ...) into catalog permissions, once per row:
/// rows already in the catalog are left alone, so running it again changes
/// nothing. Tokens lose every permission a token may not carry, and a token
/// left without any is rejected from then on.
pub async fn migrate_permission_vocabulary(store: &meta::Store) -> anyhow::Result<()> {
    let mut roles = 0usize;
    for p in store.list_permissions().await.context("list permissions")? {
        let actions: Vec<String> = p
            .actions
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_string)
            .collect();
        if !is_legacy(&actions) {
            continue;
        }
        let entries = translate_legacy(&p.repo_pattern, &actions)
            .into_iter()
            .map(|(pattern, acts)| (pattern, acts.join(",")))
            .collect();
        store
            .rewrite_permission(p.id, entries)
            .await
            .context("rewrite permission")?;
        roles += 1;
    }
    let mut tokens = 0usize;
    for t in store.list_all_tokens().await.context("list tokens")? {
        let Ok(scopes) = serde_json::from_str::<Vec<Scope>>(&t.scopes_json) else {
            continue;
        };
        if !scopes.iter().any(|s| is_legacy(&s.actions)) {
            continue;
        }
        let json = serde_json::to_string(&translate_legacy_scopes(&scopes))
            .context("encode token scopes")?;
        store
            .update_token_scopes(t.user_id, t.id, &json)
            .await
            .context("rewrite token scopes")?;
        tokens += 1;
    }
    if roles > 0 || tokens > 0 {
        tracing::info!(
            role_permissions = roles,
            tokens,
            "rewrote stored permissions into the <category>.<verb> catalog"
        );
    }
    Ok(())
}

/// Applies the declarative RBAC policy to the store. `policy_file` is the path
/// to an ArgoCD-style policy.csv (mounted from a ConfigMap); `accounts_dir`, if
/// set, is a directory of local-account password files (mounted from a Secret),
/// one file per account named after the username with the plaintext password as
/// its content. Reconciliation is authoritative for managed rows and idempotent.
///
/// When `policy_file` is empty, declarative RBAC is disabled and the store is
/// left untouched. A configured-but-missing policy file is treated as disabled
/// (a warning is logged) so a chart misconfiguration does not wipe managed rows.
pub async fn reconcile_rbac(
    store: Arc<meta::Store>,
    policy_file: &str,
    accounts_dir: &str,
) -> anyhow::Result<()> {
    if policy_file.is_empty() {
        return Ok(());
    }
    let body = match tokio::fs::read_to_string(policy_file).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(
                path = %policy_file,
                "RBAC policy file not found; skipping declarative reconciliation"
            );
            return Ok(());
        }
        Err(e) => return Err(anyhow::Error::new(e).context("read RBAC policy")),
    };

    let mut desired = parse_policy(&body).context("parse RBAC policy")?;

    desired.local_users = load_accounts(accounts_dir)
        .await
        .context("load local accounts")?;

    let counts = (
        desired.roles.len(),
        desired.group_roles.len(),
        desired.user_roles.len(),
        desired.local_users.len(),
    );
    store
        .apply_managed_rbac(desired)
        .await
        .context("apply RBAC policy")?;
    tracing::info!(
        roles = counts.0,
        group_mappings = counts.1,
        user_roles = counts.2,
        local_accounts = counts.3,
        "reconciled declarative RBAC"
    );
    Ok(())
}

/// Reads local-account passwords from a mounted Secret directory. Each regular
/// file's name is the username and its content is the plaintext password, which
/// is hashed before storage. Dotfiles (e.g. Kubernetes `..data` symlinks) are
/// skipped.
pub(crate) async fn load_accounts(dir: &str) -> anyhow::Result<Vec<meta::ManagedLocalUser>> {
    if dir.is_empty() {
        return Ok(Vec::new());
    }
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    let mut names: Vec<String> = Vec::new();
    while let Some(e) = entries.next_entry().await? {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || e.file_type().await?.is_dir() {
            continue;
        }
        names.push(name);
    }
    names.sort();
    for name in names {
        let raw = tokio::fs::read_to_string(Path::new(dir).join(&name)).await?;
        let password = raw.trim();
        if password.is_empty() {
            continue;
        }
        let hash = hash_password(password)?;
        out.push(meta::ManagedLocalUser {
            username: name,
            password_hash: hash,
            email: String::new(),
        });
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use crate::auth::*;
    use crate::testing::auth::{new_test_service, new_test_service_with};

    #[tokio::test]
    async fn reconcile_rbac_disabled() {
        let t = new_test_service().await;
        // Empty policy file path is a no-op (declarative RBAC disabled).
        reconcile_rbac(Arc::clone(&t.store), "", "")
            .await
            .expect("disabled reconcile should succeed");
        let roles = t.store.list_roles().await.unwrap();
        assert!(roles.is_empty(), "no roles expected, got {}", roles.len());
    }

    #[tokio::test]
    async fn reconcile_rbac_missing_file() {
        let t = new_test_service().await;
        let dir = tempfile::tempdir().unwrap();
        // A configured-but-missing file is tolerated (warn + skip), not an error.
        reconcile_rbac(
            Arc::clone(&t.store),
            dir.path().join("nope.csv").to_str().unwrap(),
            "",
        )
        .await
        .expect("missing file should not error");
    }

    #[tokio::test]
    async fn reconcile_rbac_end_to_end() {
        let t = new_test_service().await;

        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join("policy.csv");
        let policy = "p, readonly, repo, read, *, allow\np, dev, repo, write, team-*, allow\ng, user:ci-bot, dev\n";
        std::fs::write(&policy_path, policy).unwrap();

        let accounts_dir = dir.path().join("accounts");
        std::fs::create_dir_all(&accounts_dir).unwrap();
        std::fs::write(accounts_dir.join("ci-bot"), "s3cr3t\n").unwrap();
        // A Kubernetes-style dotfile must be ignored.
        std::fs::write(accounts_dir.join(".hidden"), "x").unwrap();

        reconcile_rbac(
            Arc::clone(&t.store),
            policy_path.to_str().unwrap(),
            accounts_dir.to_str().unwrap(),
        )
        .await
        .expect("reconcile");

        let bot = t
            .store
            .get_user_by_username("ci-bot")
            .await
            .expect("ci-bot not provisioned");
        // Local account: password hashed and stored, source local.
        assert_eq!(bot.source, "local", "ci-bot source");
        assert!(
            verify_password(&bot.password_hash, "s3cr3t"),
            "ci-bot password not set correctly"
        );
        assert!(
            t.store.get_user_by_username(".hidden").await.is_err(),
            "dotfile should not create a user"
        );
    }

    #[tokio::test]
    async fn reconcile_rbac_invalid_policy() {
        let t = new_test_service().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.csv");
        std::fs::write(&path, "p, r, repo, bogus, *, allow").unwrap();
        assert!(
            reconcile_rbac(Arc::clone(&t.store), path.to_str().unwrap(), "")
                .await
                .is_err(),
            "invalid policy should error"
        );
    }

    #[tokio::test]
    async fn default_role_grants_permissions() {
        let t = new_test_service_with(Options {
            session_secret: b"test-secret-test-secret-test-secret".to_vec(),
            default_role: "readonly".into(),
            ..Default::default()
        })
        .await;

        // Reconcile a readonly role and configure it as the default.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.csv");
        std::fs::write(&path, "p, readonly, repo, read, *, allow\n").unwrap();
        reconcile_rbac(Arc::clone(&t.store), path.to_str().unwrap(), "")
            .await
            .expect("reconcile");

        // A brand-new local user with no explicit roles inherits readonly.
        let hash = hash_password("pw").unwrap();
        t.store
            .create_user(crate::meta::User {
                username: "nobody".into(),
                password_hash: hash,
                source: crate::meta::SOURCE_LOCAL.into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let p = t
            .svc
            .principal_from_password("nobody", "pw")
            .await
            .expect("resolve")
            .expect("resolve");
        assert!(
            p.can("anyrepo", REPO_READ),
            "default role should grant read"
        );
        assert!(
            !p.can("anyrepo", REPO_WRITE),
            "default role must not grant write"
        );
    }

    /// Rows written before the catalog keep working after an upgrade: each one is
    /// rewritten into what it used to grant, tokens lose what they may not carry,
    /// and a second run changes nothing.
    #[tokio::test]
    async fn migrate_permission_vocabulary_rewrites_legacy_rows() {
        let t = new_test_service().await;
        let role = |name: &str| crate::meta::Role {
            name: name.into(),
            ..Default::default()
        };
        let admins = t.store.create_role(role("admins")).await.unwrap();
        let auditors = t.store.create_role(role("auditors")).await.unwrap();
        let current = t.store.create_role(role("current")).await.unwrap();
        for (role_id, pattern, actions) in [
            (admins.id, "*", "admin"),
            (auditors.id, "*", "read,audit"),
            (current.id, "lib-*", "repo.read"),
        ] {
            t.store
                .add_permission(crate::meta::Permission {
                    role_id,
                    repo_pattern: pattern.into(),
                    actions: actions.into(),
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        let user = t
            .store
            .create_user(crate::meta::User {
                username: "ci".into(),
                source: crate::meta::SOURCE_LOCAL.into(),
                ..Default::default()
            })
            .await
            .unwrap();
        t.store
            .create_token(crate::meta::Token {
                user_id: user.id,
                name: "ci".into(),
                hash: "h".into(),
                scopes_json: r#"[{"repo_pattern":"maven-*","actions":["read","approve"]}]"#.into(),
                ..Default::default()
            })
            .await
            .unwrap();

        migrate_permission_vocabulary(&t.store)
            .await
            .expect("migrate");
        let snapshot = async || {
            let mut perms: Vec<(i64, String, String)> = t
                .store
                .list_permissions()
                .await
                .unwrap()
                .into_iter()
                .map(|p| (p.role_id, p.repo_pattern, p.actions))
                .collect();
            perms.sort();
            let tokens = t.store.list_all_tokens().await.unwrap();
            (perms, tokens[0].scopes_json.clone())
        };
        let first = snapshot().await;
        assert_eq!(
            first.0,
            vec![
                (admins.id, String::new(), "*".to_string()),
                (
                    auditors.id,
                    String::new(),
                    "audit.read,user.read,role.read,approval.read,notification.read,insights.read"
                        .to_string()
                ),
                (auditors.id, "*".to_string(), "repo.read".to_string()),
                (current.id, "lib-*".to_string(), "repo.read".to_string()),
            ]
        );
        assert_eq!(
            first.1,
            r#"[{"repo_pattern":"maven-*","actions":["repo.read"]}]"#
        );

        migrate_permission_vocabulary(&t.store)
            .await
            .expect("migrate again");
        assert_eq!(snapshot().await, first, "a second run changes nothing");
    }
}
