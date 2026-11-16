//! Permission catalog and the resolved [`Principal`] every authorization check
//! runs against.
//!
//! A permission is named `<category>.<verb>`. Repository permissions
//! (`repo.*`) apply to repositories matching a glob pattern. Every other
//! permission is global and carries no pattern. `*` grants everything.

use crate::meta;

/// Pull, browse and search a repository.
pub const REPO_READ: &str = "repo.read";
/// Push, upload and yank into a repository.
pub const REPO_WRITE: &str = "repo.write";
/// Delete artifacts and publications from a repository.
pub const REPO_DELETE: &str = "repo.delete";
/// Decide package approvals and version denies on a repository.
pub const REPO_APPROVE: &str = "repo.approve";
/// Edit the security-policy subset of a repository's configuration. Upstream URL
/// and credentials stay with [`REPO_ADMIN`].
pub const REPO_SECURITY: &str = "repo.security";
/// Every `repo.*` permission on matching repositories, plus repository settings,
/// labels, bulk deletes and creating a repository whose name matches.
pub const REPO_ADMIN: &str = "repo.admin";
/// List users and their tokens.
pub const USER_READ: &str = "user.read";
/// Manage users, their role assignments and tokens, and impersonate them.
pub const USER_WRITE: &str = "user.write";
/// List roles, their permissions and group mappings.
pub const ROLE_READ: &str = "role.read";
/// Manage roles, their permissions and group mappings.
pub const ROLE_WRITE: &str = "role.write";
/// Read repository audit logs, permissions and token listings.
pub const AUDIT_READ: &str = "audit.read";
/// Read the package approval queue and the version-deny list.
pub const APPROVAL_READ: &str = "approval.read";
/// List notification receivers.
pub const NOTIFICATION_READ: &str = "notification.read";
/// Manage notification receivers and send test notifications.
pub const NOTIFICATION_WRITE: &str = "notification.write";
/// Read coverage settings and alarm history.
pub const COVERAGE_READ: &str = "coverage.read";
/// Change coverage settings, mutes and run scans.
pub const COVERAGE_WRITE: &str = "coverage.write";
/// See every insights finding, including account and token findings.
pub const INSIGHTS_READ: &str = "insights.read";
/// Refresh insights, change their settings, acknowledge checks and send alarms.
pub const INSIGHTS_WRITE: &str = "insights.write";
/// Read HA and storage status.
pub const SYSTEM_READ: &str = "system.read";
/// Step down the HA leader and edit the site-wide announcement.
pub const SYSTEM_WRITE: &str = "system.write";
/// Every permission on every repository.
pub const ALL: &str = "*";

/// Whether a permission applies to repositories or to the whole instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionScope {
    Repo,
    Global,
}

/// One entry of the permission catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionDef {
    pub name: &'static str,
    pub scope: PermissionScope,
    /// Whether a personal access token may carry it. Decisions (approve,
    /// security), administration and every global write are session-only.
    pub token: bool,
}

const fn repo(name: &'static str, token: bool) -> PermissionDef {
    PermissionDef {
        name,
        scope: PermissionScope::Repo,
        token,
    }
}

const fn global(name: &'static str, token: bool) -> PermissionDef {
    PermissionDef {
        name,
        scope: PermissionScope::Global,
        token,
    }
}

/// Every grantable permission, `*` included.
pub const PERMISSIONS: &[PermissionDef] = &[
    repo(REPO_READ, true),
    repo(REPO_WRITE, true),
    repo(REPO_DELETE, true),
    repo(REPO_APPROVE, false),
    repo(REPO_SECURITY, false),
    repo(REPO_ADMIN, false),
    global(USER_READ, true),
    global(USER_WRITE, false),
    global(ROLE_READ, true),
    global(ROLE_WRITE, false),
    global(AUDIT_READ, true),
    global(APPROVAL_READ, true),
    global(NOTIFICATION_READ, true),
    global(NOTIFICATION_WRITE, false),
    global(COVERAGE_READ, true),
    global(COVERAGE_WRITE, false),
    global(INSIGHTS_READ, true),
    global(INSIGHTS_WRITE, false),
    global(SYSTEM_READ, true),
    global(SYSTEM_WRITE, false),
    global(ALL, false),
];

/// Looks up a permission by name.
pub fn permission(name: &str) -> Option<&'static PermissionDef> {
    PERMISSIONS.iter().find(|p| p.name == name)
}

/// Reports whether `name` is a repository permission.
pub fn is_repo_permission(name: &str) -> bool {
    permission(name).is_some_and(|p| p.scope == PermissionScope::Repo)
}

/// Reports whether a personal access token may carry `name`.
pub fn token_allowed(name: &str) -> bool {
    permission(name).is_some_and(|p| p.token)
}

/// Checks one permission entry (a role permission or a token scope). Repository
/// permissions need a pattern, global ones must have none, and the two kinds
/// never share an entry. `Err` carries a message for the caller.
pub fn validate_entry(pattern: &str, actions: &[String], for_token: bool) -> Result<(), String> {
    if actions.is_empty() {
        return Err("at least one permission required".into());
    }
    let mut repo_kind = false;
    let mut global_kind = false;
    for a in actions {
        let Some(def) = permission(a) else {
            return Err(format!("unknown permission: {a}"));
        };
        if for_token && !def.token {
            return Err(format!("permission {a} cannot be carried by a token"));
        }
        match def.scope {
            PermissionScope::Repo => repo_kind = true,
            PermissionScope::Global => global_kind = true,
        }
    }
    if repo_kind && global_kind {
        return Err("repository and global permissions must be separate entries".into());
    }
    let pattern = pattern.trim();
    if repo_kind && pattern.is_empty() {
        return Err("repo_pattern required for repository permissions".into());
    }
    if global_kind && !pattern.is_empty() {
        return Err("repo_pattern must be empty for global permissions".into());
    }
    Ok(())
}

/// One entry of a personal access token's scope: permissions on repositories
/// matching a glob pattern, or global permissions with an empty pattern.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Scope {
    #[serde(rename = "repo_pattern", default)]
    pub repo_pattern: String,
    #[serde(default)]
    pub actions: Vec<String>,
}

/// An authenticated identity with resolved effective permissions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Principal {
    pub username: String,
    pub source: String,
    /// The administrator acting as this principal, empty for an ordinary
    /// session. The principal's permissions are always the impersonated user's
    /// own, never the administrator's, so impersonation never widens access;
    /// the field exists for attribution and for stopping the session.
    pub impersonator: String,
    /// The union of permissions granted via the principal's roles.
    pub(crate) perms: Vec<meta::Permission>,
    /// When true, [`Principal::token_scopes`] further restricts `perms`
    /// (intersection), and an empty scope list grants nothing.
    pub(crate) via_token: bool,
    pub(crate) token_scopes: Vec<Scope>,
}

impl Principal {
    /// Reports whether the principal holds `*`. Never true for a token.
    pub fn is_admin(&self) -> bool {
        self.has(ALL)
    }

    /// Reports whether the principal authenticated with a personal access token.
    pub const fn via_token(&self) -> bool {
        self.via_token
    }

    /// Reports whether the principal may use `perm` on `repo`. Global
    /// permissions ignore `repo`. An empty `repo` with a repository permission
    /// asks for it on every repository (only a `*` pattern matches).
    pub fn can(&self, repo: &str, perm: &str) -> bool {
        let Some(def) = permission(perm) else {
            return false;
        };
        let roles = self
            .perms
            .iter()
            .any(|p| entry_grants(&p.repo_pattern, split_csv(&p.actions), def, Some(repo)));
        roles && self.token_allows(def, Some(repo))
    }

    /// Reports whether the principal holds a global permission. For a repository
    /// permission it reports whether the principal holds it on at least one
    /// pattern, which gates surfaces such as the approvals queue whose handlers
    /// then check each repository.
    pub fn has(&self, perm: &str) -> bool {
        let Some(def) = permission(perm) else {
            return false;
        };
        let roles = self
            .perms
            .iter()
            .any(|p| entry_grants(&p.repo_pattern, split_csv(&p.actions), def, None));
        roles && self.token_allows(def, None)
    }

    /// Reports whether the principal holds `perm` on everything an entry with
    /// `pattern` would grant, the test for handing that grant to someone else.
    /// A repository pattern is covered when one of the principal's patterns
    /// matches it as written (`team-*` covers `team-a-*`, never `*`).
    pub fn covers(&self, pattern: &str, perm: &str) -> bool {
        match permission(perm) {
            Some(def) if def.name == ALL => self.is_admin(),
            Some(def) if def.scope == PermissionScope::Repo => {
                !pattern.is_empty() && self.can(pattern, perm)
            }
            Some(def) => self.has(def.name),
            None => false,
        }
    }

    /// Reports whether every entry in `perms` is covered, so a role or user
    /// holding them grants nothing beyond the principal's own reach.
    pub fn covers_all(&self, perms: &[meta::Permission]) -> bool {
        perms
            .iter()
            .all(|p| split_csv(&p.actions).all(|a| self.covers(&p.repo_pattern, a)))
    }

    /// The permission names the principal holds anywhere, in catalog order, for
    /// the UI to decide what to show.
    pub fn permission_names(&self) -> Vec<&'static str> {
        PERMISSIONS
            .iter()
            .filter(|d| self.has(d.name))
            .map(|d| d.name)
            .collect()
    }

    fn token_allows(&self, def: &PermissionDef, repo: Option<&str>) -> bool {
        if !self.via_token {
            return true;
        }
        if !def.token {
            return false;
        }
        self.token_scopes.iter().any(|s| {
            s.actions.iter().any(|a| a == def.name)
                && (def.scope == PermissionScope::Global
                    || repo.is_none_or(|r| match_glob(&s.repo_pattern, r)))
        })
    }
}

/// Reports whether one role permission entry grants `def`. `repo` is `None`
/// when the question is "on any repository".
fn entry_grants<'a>(
    pattern: &str,
    mut actions: impl Iterator<Item = &'a str>,
    def: &PermissionDef,
    repo: Option<&str>,
) -> bool {
    actions.any(|a| {
        if a == ALL {
            return true;
        }
        match def.scope {
            PermissionScope::Global => a == def.name,
            PermissionScope::Repo => {
                (a == def.name || a == REPO_ADMIN) && repo.is_none_or(|r| match_glob(pattern, r))
            }
        }
    })
}

fn split_csv(csv: &str) -> impl Iterator<Item = &str> {
    csv.split(',').map(str::trim).filter(|a| !a.is_empty())
}

/// Permissions a legacy `audit` action stood for: it opened every read-only
/// administrative surface regardless of its pattern.
const LEGACY_AUDIT: &[&str] = &[
    AUDIT_READ,
    USER_READ,
    ROLE_READ,
    APPROVAL_READ,
    NOTIFICATION_READ,
    INSIGHTS_READ,
];

/// Permissions a legacy `security` action granted beyond its repositories: it
/// opened the insights settings and acknowledgements.
const LEGACY_SECURITY_GLOBAL: &[&str] = &[INSIGHTS_READ, INSIGHTS_WRITE];

/// Reports whether an action list still uses the pre-catalog vocabulary.
pub fn is_legacy(actions: &[String]) -> bool {
    actions.iter().any(|a| permission(a).is_none())
}

/// Translates one entry written in the pre-catalog vocabulary (`read`, `write`,
/// `delete`, `approve`, `audit`, `security`, `admin`) into catalog entries: the
/// repository permissions on `pattern` and the global permissions on an empty
/// pattern. `admin` on `*` was the instance administrator and becomes `*`.
/// Names already in the catalog pass through, and unknown names are dropped.
pub fn translate_legacy(pattern: &str, actions: &[String]) -> Vec<(String, Vec<String>)> {
    let mut repo_perms: Vec<&'static str> = Vec::new();
    let mut global_perms: Vec<&'static str> = Vec::new();
    let push = |list: &mut Vec<&'static str>, name: &'static str| {
        if !list.contains(&name) {
            list.push(name);
        }
    };
    for a in actions {
        match a.as_str() {
            "read" => push(&mut repo_perms, REPO_READ),
            "write" => push(&mut repo_perms, REPO_WRITE),
            "delete" => push(&mut repo_perms, REPO_DELETE),
            "approve" => push(&mut repo_perms, REPO_APPROVE),
            "security" => {
                push(&mut repo_perms, REPO_SECURITY);
                for g in LEGACY_SECURITY_GLOBAL {
                    push(&mut global_perms, g);
                }
            }
            "audit" => {
                for g in LEGACY_AUDIT {
                    push(&mut global_perms, g);
                }
            }
            "admin" if pattern == "*" => push(&mut global_perms, ALL),
            "admin" => push(&mut repo_perms, REPO_ADMIN),
            other => {
                if let Some(def) = permission(other) {
                    match def.scope {
                        PermissionScope::Repo => push(&mut repo_perms, def.name),
                        PermissionScope::Global => push(&mut global_perms, def.name),
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    if global_perms.contains(&ALL) {
        return vec![(String::new(), vec![ALL.to_string()])];
    }
    if !repo_perms.is_empty() && !pattern.is_empty() {
        out.push((
            pattern.to_string(),
            repo_perms.iter().map(|s| (*s).to_string()).collect(),
        ));
    }
    if !global_perms.is_empty() {
        out.push((
            String::new(),
            global_perms.iter().map(|s| (*s).to_string()).collect(),
        ));
    }
    out
}

/// Translates a token's scope list into the catalog and drops every permission
/// a token may not carry. A list that ends up empty grants nothing.
pub fn translate_legacy_scopes(scopes: &[Scope]) -> Vec<Scope> {
    let mut out: Vec<Scope> = Vec::new();
    for s in scopes {
        for (pattern, actions) in translate_legacy(&s.repo_pattern, &s.actions) {
            let actions: Vec<String> = actions.into_iter().filter(|a| token_allowed(a)).collect();
            if actions.is_empty() {
                continue;
            }
            if let Some(existing) = out.iter_mut().find(|e| e.repo_pattern == pattern) {
                for a in actions {
                    if !existing.actions.contains(&a) {
                        existing.actions.push(a);
                    }
                }
            } else {
                out.push(Scope {
                    repo_pattern: pattern,
                    actions,
                });
            }
        }
    }
    out
}

/// Reports whether a permission's repo pattern matches a repository name, using
/// the same glob semantics as authorization. Exported so the API can list which
/// roles apply to a given repository.
pub fn match_repo_pattern(pattern: &str, name: &str) -> bool {
    match_glob(pattern, name)
}

/// Matches a repository name against a pattern that may contain `*` wildcards.
/// Repository names contain no slashes, so `*` matches any run of characters.
/// An empty repo only matches a `*` pattern (global checks).
pub(crate) fn match_glob(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if name.is_empty() {
        return false;
    }
    glob_match(pattern, name)
}

/// A small `*`-only glob matcher using the canonical two-pointer algorithm with star
/// backtracking.
const fn glob_match(pattern: &str, s: &str) -> bool {
    let (pattern, s) = (pattern.as_bytes(), s.as_bytes());
    let (mut sx, mut px) = (0usize, 0usize);
    let (mut star_idx, mut s_tmp) = (usize::MAX, 0usize);
    while sx < s.len() {
        if px < pattern.len() && pattern[px] == s[sx] {
            sx += 1;
            px += 1;
        } else if px < pattern.len() && pattern[px] == b'*' {
            star_idx = px;
            s_tmp = sx;
            px += 1;
        } else if star_idx != usize::MAX {
            px = star_idx + 1;
            s_tmp += 1;
            sx = s_tmp;
        } else {
            return false;
        }
    }
    while px < pattern.len() && pattern[px] == b'*' {
        px += 1;
    }
    px == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(pattern: &str, actions: &str) -> meta::Permission {
        meta::Permission {
            repo_pattern: pattern.into(),
            actions: actions.into(),
            ..Default::default()
        }
    }

    fn strs(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn catalog_names_are_category_dot_verb() {
        for d in PERMISSIONS {
            if d.name == ALL {
                continue;
            }
            let (cat, verb) = d.name.split_once('.').expect("category.verb");
            assert!(!cat.is_empty() && !verb.is_empty(), "{}", d.name);
            assert_eq!(
                d.scope == PermissionScope::Repo,
                cat == "repo",
                "{}: only repo.* is repository scoped",
                d.name
            );
        }
    }

    #[test]
    fn role_checks() {
        let p = Principal {
            perms: vec![
                role("maven-*", "repo.read,repo.write"),
                role("npm-*", "repo.admin"),
                role("", "user.read"),
            ],
            ..Default::default()
        };
        let cases = [
            ("maven-central", REPO_READ, true),
            ("maven-central", REPO_DELETE, false),
            ("pypi", REPO_READ, false),
            ("npm-public", REPO_DELETE, true),
            ("npm-public", REPO_APPROVE, true),
            ("maven-central", USER_READ, true),
            ("", USER_READ, true),
            ("", USER_WRITE, false),
            ("", REPO_READ, false),
            ("anything", "read", false),
        ];
        for (repo, perm, want) in cases {
            assert_eq!(p.can(repo, perm), want, "can({repo:?}, {perm:?})");
        }
        assert!(p.has(REPO_READ) && p.has(REPO_SECURITY) && !p.has(ROLE_READ));
        assert!(!p.is_admin());
    }

    #[test]
    fn superuser_holds_everything() {
        let p = Principal {
            perms: vec![role("", ALL)],
            ..Default::default()
        };
        assert!(p.is_admin());
        assert!(p.can("", REPO_READ) && p.can("x", REPO_ADMIN) && p.has(SYSTEM_WRITE));
        assert_eq!(p.permission_names().len(), PERMISSIONS.len());
    }

    #[test]
    fn token_checks() {
        let full = vec![role("", ALL)];
        let token = |scopes: Vec<Scope>| Principal {
            perms: full.clone(),
            via_token: true,
            token_scopes: scopes,
            ..Default::default()
        };
        let scope = |pattern: &str, actions: &[&str]| Scope {
            repo_pattern: pattern.into(),
            actions: strs(actions),
        };

        let empty = token(Vec::new());
        assert!(!empty.can("maven", REPO_READ) && !empty.has(USER_READ));

        let scoped = token(vec![
            scope("maven-*", &[REPO_READ]),
            scope("", &[AUDIT_READ]),
        ]);
        assert!(scoped.can("maven-central", REPO_READ));
        assert!(!scoped.can("npm", REPO_READ));
        assert!(!scoped.can("maven-central", REPO_WRITE));
        assert!(scoped.has(AUDIT_READ) && !scoped.has(USER_READ));
        assert!(!scoped.is_admin(), "a token never holds *");

        let smuggled = token(vec![
            scope("*", &[REPO_APPROVE, REPO_SECURITY, REPO_ADMIN, ALL]),
            scope("", &[USER_WRITE]),
        ]);
        for perm in [REPO_APPROVE, REPO_SECURITY, REPO_ADMIN, REPO_READ] {
            assert!(!smuggled.can("maven", perm), "{perm} via a stored scope");
        }
        assert!(!smuggled.has(USER_WRITE) && !smuggled.is_admin());

        let narrow_role = Principal {
            perms: vec![role("maven-*", "repo.read")],
            via_token: true,
            token_scopes: vec![scope("*", &[REPO_READ, REPO_WRITE])],
            ..Default::default()
        };
        assert!(narrow_role.can("maven-a", REPO_READ));
        assert!(
            !narrow_role.can("maven-a", REPO_WRITE),
            "scope never widens the role"
        );
        assert!(!narrow_role.can("npm", REPO_READ));
    }

    #[test]
    fn covers_never_exceeds_own_reach() {
        let p = Principal {
            perms: vec![
                role("team-*", "repo.admin"),
                role("", "role.write,user.read"),
            ],
            ..Default::default()
        };
        assert!(p.covers("team-a-*", REPO_WRITE));
        assert!(p.covers("team-x", REPO_ADMIN));
        assert!(!p.covers("*", REPO_READ), "team-* does not cover *");
        assert!(!p.covers("other-*", REPO_READ));
        assert!(p.covers("", ROLE_WRITE) && p.covers("", USER_READ));
        assert!(!p.covers("", USER_WRITE));
        assert!(!p.covers("", ALL));
        assert!(p.covers_all(&[
            role("team-b", "repo.read,repo.delete"),
            role("", "user.read")
        ]));
        assert!(!p.covers_all(&[role("", "*")]));
        let admin = Principal {
            perms: vec![role("", ALL)],
            ..Default::default()
        };
        assert!(admin.covers("", ALL) && admin.covers("*", REPO_ADMIN));
    }

    #[test]
    fn entry_validation() {
        let ok = |p: &str, a: &[&str], t: bool| validate_entry(p, &strs(a), t).is_ok();
        assert!(ok("maven-*", &[REPO_READ, REPO_WRITE], true));
        assert!(ok("", &[USER_READ, AUDIT_READ], true));
        assert!(ok("", &[ALL], false));
        assert!(
            !ok("", &[REPO_READ], false),
            "repo permission needs a pattern"
        );
        assert!(
            !ok("*", &[USER_READ], false),
            "global permission takes no pattern"
        );
        assert!(!ok("*", &[REPO_READ, USER_READ], false), "kinds never mix");
        assert!(!ok("*", &[REPO_APPROVE], true), "approve is session-only");
        assert!(
            !ok("", &[USER_WRITE], true),
            "global writes are session-only"
        );
        assert!(!ok("", &[ALL], true));
        assert!(!ok("*", &["read"], false), "legacy names are rejected");
        assert!(!ok("*", &[], false));
    }

    #[test]
    fn legacy_translation() {
        let t = |p: &str, a: &[&str]| translate_legacy(p, &strs(a));
        assert_eq!(
            t("maven-*", &["read", "write"]),
            vec![("maven-*".into(), strs(&[REPO_READ, REPO_WRITE]))]
        );
        assert_eq!(
            t("*", &["admin", "read"]),
            vec![(String::new(), strs(&[ALL]))]
        );
        assert_eq!(
            t("npm-*", &["admin"]),
            vec![("npm-*".into(), strs(&[REPO_ADMIN]))]
        );
        assert_eq!(
            t("*", &["audit", "approve"]),
            vec![
                ("*".into(), strs(&[REPO_APPROVE])),
                (String::new(), strs(LEGACY_AUDIT)),
            ]
        );
        assert_eq!(
            t("lib-*", &["security"]),
            vec![
                ("lib-*".into(), strs(&[REPO_SECURITY])),
                (String::new(), strs(LEGACY_SECURITY_GLOBAL)),
            ]
        );
        assert_eq!(
            t("x-*", &[REPO_READ]),
            vec![("x-*".into(), strs(&[REPO_READ]))],
            "catalog names pass through"
        );
        assert_eq!(t("x", &["bogus"]), Vec::new());
        assert!(is_legacy(&strs(&["read"])) && !is_legacy(&strs(&[REPO_READ])));
    }

    #[test]
    fn legacy_scope_translation_drops_session_only() {
        let scopes = vec![
            Scope {
                repo_pattern: "maven-*".into(),
                actions: strs(&["read", "approve", "audit"]),
            },
            Scope {
                repo_pattern: "*".into(),
                actions: strs(&["admin"]),
            },
        ];
        let got = translate_legacy_scopes(&scopes);
        assert_eq!(
            got,
            vec![
                Scope {
                    repo_pattern: "maven-*".into(),
                    actions: strs(&[REPO_READ]),
                },
                Scope {
                    repo_pattern: String::new(),
                    actions: strs(&[
                        AUDIT_READ,
                        USER_READ,
                        ROLE_READ,
                        APPROVAL_READ,
                        NOTIFICATION_READ,
                        INSIGHTS_READ
                    ]),
                },
            ]
        );
        assert_eq!(
            translate_legacy_scopes(&[Scope {
                repo_pattern: "*".into(),
                actions: strs(&["admin"]),
            }]),
            Vec::new()
        );
    }
}
