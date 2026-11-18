//! Resolves the AWS account id and (best-effort) account alias.
//!
//! Both are looked up once at start-up and cached for the lifetime of the
//! process. Failures are non-fatal — Slack messages are still sent, just
//! without the alias.

use aws_sdk_sts::error::DisplayErrorContext;
use tracing::warn;

#[derive(Debug, Clone, Default)]
pub struct AccountIdentity {
    pub account_id: Option<String>,
    pub alias: Option<String>,
}

impl AccountIdentity {
    pub async fn resolve() -> Self {
        let cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .load()
            .await;
        Self::resolve_with(
            &aws_sdk_sts::Client::new(&cfg),
            &aws_sdk_iam::Client::new(&cfg),
        )
        .await
    }

    pub async fn resolve_with(sts: &aws_sdk_sts::Client, iam: &aws_sdk_iam::Client) -> Self {
        let account_id = match sts.get_caller_identity().send().await {
            Ok(out) => out.account,
            Err(e) => {
                warn!(error = %DisplayErrorContext(&e), "failed to resolve account id via STS");
                None
            }
        };

        let alias = match iam.list_account_aliases().send().await {
            Ok(out) => out.account_aliases.into_iter().next(),
            Err(e) => {
                warn!(error = %DisplayErrorContext(&e), "failed to resolve account alias via IAM (alias display disabled)");
                None
            }
        };

        Self { account_id, alias }
    }

    /// "alias (id)" if both known, else id, alias, or None.
    pub fn display(&self) -> Option<String> {
        match (self.alias.as_deref(), self.account_id.as_deref()) {
            (Some(a), Some(i)) => Some(format!("{a} ({i})")),
            (Some(a), None) => Some(a.to_string()),
            (None, Some(i)) => Some(i.to_string()),
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(alias: Option<&str>, account: Option<&str>) -> AccountIdentity {
        AccountIdentity {
            account_id: account.map(str::to_string),
            alias: alias.map(str::to_string),
        }
    }

    use aws_smithy_http_client::test_util::{ReplayEvent, StaticReplayClient};
    use aws_smithy_types::body::SdkBody;

    const CALLER_IDENTITY: &str = r#"<GetCallerIdentityResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/"><GetCallerIdentityResult><Arn>arn:aws:iam::123456789012:user/test</Arn><UserId>AIDAEXAMPLE</UserId><Account>123456789012</Account></GetCallerIdentityResult><ResponseMetadata><RequestId>r</RequestId></ResponseMetadata></GetCallerIdentityResponse>"#;
    const ACCESS_DENIED: &str = r"<ErrorResponse><Error><Type>Sender</Type><Code>AccessDenied</Code><Message>denied</Message></Error><RequestId>r</RequestId></ErrorResponse>";

    fn aliases(names: &[&str]) -> String {
        let mut members = String::new();
        for name in names {
            members.push_str("<member>");
            members.push_str(name);
            members.push_str("</member>");
        }
        format!(
            r#"<ListAccountAliasesResponse xmlns="https://iam.amazonaws.com/doc/2010-05-08/"><ListAccountAliasesResult><AccountAliases>{members}</AccountAliases><IsTruncated>false</IsTruncated></ListAccountAliasesResult><ResponseMetadata><RequestId>r</RequestId></ResponseMetadata></ListAccountAliasesResponse>"#
        )
    }

    fn replay(status: u16, body: &str) -> StaticReplayClient {
        StaticReplayClient::new(vec![ReplayEvent::new(
            http::Request::builder()
                .uri("https://example.com/")
                .body(SdkBody::empty())
                .unwrap(),
            http::Response::builder()
                .status(status)
                .header("content-type", "text/xml")
                .body(SdkBody::from(body.to_string()))
                .unwrap(),
        )])
    }

    fn sts(status: u16, body: &str) -> aws_sdk_sts::Client {
        aws_sdk_sts::Client::from_conf(
            aws_sdk_sts::Config::builder()
                .behavior_version(aws_sdk_sts::config::BehaviorVersion::latest())
                .region(aws_sdk_sts::config::Region::new("us-east-1"))
                .credentials_provider(aws_sdk_sts::config::Credentials::for_tests())
                .http_client(replay(status, body))
                .build(),
        )
    }

    fn iam(status: u16, body: &str) -> aws_sdk_iam::Client {
        aws_sdk_iam::Client::from_conf(
            aws_sdk_iam::Config::builder()
                .behavior_version(aws_sdk_iam::config::BehaviorVersion::latest())
                .region(aws_sdk_iam::config::Region::new("us-east-1"))
                .credentials_provider(aws_sdk_iam::config::Credentials::for_tests())
                .http_client(replay(status, body))
                .build(),
        )
    }

    #[tokio::test]
    async fn resolve_reads_account_id_and_first_alias() {
        let identity = AccountIdentity::resolve_with(
            &sts(200, CALLER_IDENTITY),
            &iam(200, &aliases(&["prod", "ignored"])),
        )
        .await;
        assert_eq!(identity.account_id.as_deref(), Some("123456789012"));
        assert_eq!(identity.alias.as_deref(), Some("prod"));
        assert_eq!(identity.display().as_deref(), Some("prod (123456789012)"));
    }

    #[tokio::test]
    async fn resolve_without_alias_shows_the_account_id() {
        let identity =
            AccountIdentity::resolve_with(&sts(200, CALLER_IDENTITY), &iam(200, &aliases(&[])))
                .await;
        assert_eq!(identity.alias, None);
        assert_eq!(identity.display().as_deref(), Some("123456789012"));
    }

    #[tokio::test]
    async fn resolve_tolerates_denied_iam() {
        let identity =
            AccountIdentity::resolve_with(&sts(200, CALLER_IDENTITY), &iam(403, ACCESS_DENIED))
                .await;
        assert_eq!(identity.account_id.as_deref(), Some("123456789012"));
        assert_eq!(identity.alias, None);
    }

    #[tokio::test]
    async fn resolve_tolerates_denied_sts() {
        let identity =
            AccountIdentity::resolve_with(&sts(403, ACCESS_DENIED), &iam(200, &aliases(&["prod"])))
                .await;
        assert_eq!(identity.account_id, None);
        assert_eq!(identity.display().as_deref(), Some("prod"));
    }

    #[test]
    fn display_variants() {
        assert_eq!(
            id(Some("prod"), Some("123")).display().as_deref(),
            Some("prod (123)")
        );
        assert_eq!(id(Some("prod"), None).display().as_deref(), Some("prod"));
        assert_eq!(id(None, Some("123")).display().as_deref(), Some("123"));
        assert_eq!(id(None, None).display(), None);
    }
}
