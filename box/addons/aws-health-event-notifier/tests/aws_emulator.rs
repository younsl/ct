//! STS, IAM, and AWS Health calls against a local AWS Emulator.
//!
//! Ignored by default. `make test-emulator` starts the emulator and runs these
//! with `--ignored`. The emulated account has no Business or Enterprise Support
//! plan, so every AWS Health call ends in `SubscriptionRequiredException`.

use aws_health_event_notifier::aws::account::AccountIdentity;
use aws_health_event_notifier::aws::health::{EventSummary, HealthClient};
use aws_health_event_notifier::config::{RunArgs, SlackArgs};
use aws_health_event_notifier::server;
use chrono::{Duration, Utc};
use clap::Parser;

const IGNORE: &str = "needs AWS Emulator, run `make test-emulator`";
const EMULATOR_ACCOUNT: &str = "000000000000";
const ALIAS: &str = "ahen-emulator";
const DENIED: &str = "SubscriptionRequiredException";

#[derive(Parser)]
struct TestCli {
    #[command(flatten)]
    slack: SlackArgs,
    #[command(flatten)]
    run: RunArgs,
}

fn require_emulator() {
    assert!(
        std::env::var("AWS_ENDPOINT_URL").is_ok(),
        "AWS_ENDPOINT_URL is not set. {IGNORE}"
    );
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

async fn health() -> HealthClient {
    require_emulator();
    HealthClient::from_env("en").await.expect("HealthClient")
}

fn arn_only(arn: &str) -> EventSummary {
    EventSummary {
        arn: arn.into(),
        service: None,
        event_type_code: None,
        event_type_category: None,
        region: None,
        start_time: None,
        end_time: None,
        last_updated_time: None,
        status_code: None,
    }
}

fn assert_denied(what: &str, err: &anyhow::Error) {
    let msg = format!("{err:#}");
    assert!(msg.contains(DENIED), "{what}: {msg}");
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn account_identity_resolves_the_id_and_follows_the_alias() {
    require_emulator();
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let iam = aws_sdk_iam::Client::new(&config);
    let _ = iam.delete_account_alias().account_alias(ALIAS).send().await;

    let bare = AccountIdentity::resolve().await;
    assert_eq!(bare.account_id.as_deref(), Some(EMULATOR_ACCOUNT));
    assert_eq!(bare.alias, None);
    assert_eq!(bare.display().as_deref(), Some(EMULATOR_ACCOUNT));

    iam.create_account_alias()
        .account_alias(ALIAS)
        .send()
        .await
        .expect("CreateAccountAlias");
    let named = AccountIdentity::resolve().await;
    assert_eq!(named.alias.as_deref(), Some(ALIAS));
    assert_eq!(
        named.display(),
        Some(format!("{ALIAS} ({EMULATOR_ACCOUNT})"))
    );

    iam.delete_account_alias()
        .account_alias(ALIAS)
        .send()
        .await
        .expect("DeleteAccountAlias");
    assert_eq!(AccountIdentity::resolve().await.alias, None);
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn event_listing_reports_the_missing_support_plan() {
    let aws = health().await;
    let services = ["EC2".to_string()];
    let categories = ["issue".to_string()];

    let err = aws
        .list_events(Utc::now() - Duration::hours(1), Utc::now(), &[], &[])
        .await
        .expect_err("DescribeEvents");
    assert_denied("list_events", &err);
    assert!(format!("{err:#}").contains("describe_events page failed"));

    let err = aws
        .list_events(
            Utc::now() - Duration::hours(1),
            Utc::now(),
            &services,
            &categories,
        )
        .await
        .expect_err("DescribeEvents with filters");
    assert_denied("list_events with filters", &err);

    let err = aws
        .list_upcoming(Duration::hours(24), &services, &categories)
        .await
        .expect_err("DescribeEvents upcoming");
    assert_denied("list_upcoming", &err);
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn event_hydration_reports_the_missing_support_plan() {
    let err = health()
        .await
        .hydrate(arn_only(
            "arn:aws:health:us-east-1::event/EC2/AWS_EC2_OPERATIONAL_ISSUE/example",
        ))
        .await
        .expect_err("DescribeEventDetails");
    assert_denied("hydrate", &err);
    assert!(format!("{err:#}").contains("describe_event_details"));
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn catalog_lookups_report_the_missing_support_plan() {
    let aws = health().await;

    let err = aws
        .lookup_service_codes(&["EC2".to_string()])
        .await
        .expect_err("DescribeEventTypes by service");
    assert_denied("lookup_service_codes", &err);

    let err = aws
        .lookup_event_type_codes(&["AWS_EC2_OPERATIONAL_ISSUE".to_string()])
        .await
        .expect_err("DescribeEventTypes by code");
    assert_denied("lookup_event_type_codes", &err);
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn empty_catalog_lookups_make_no_api_call() {
    let aws = health().await;
    assert!(
        aws.lookup_service_codes(&[])
            .await
            .expect("no call")
            .is_empty()
    );
    assert!(
        aws.lookup_event_type_codes(&[])
            .await
            .expect("no call")
            .is_empty()
    );
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn startup_aborts_with_the_service_error_code() {
    require_emulator();
    let cli = TestCli::parse_from([
        "test",
        "--slack-webhook-url",
        "https://example.com/webhook",
        "--admin-addr",
        "127.0.0.1:0",
        "--allow-services",
        "EC2",
    ]);

    let err = server::run(cli.slack, cli.run)
        .await
        .expect_err("startup must fail without a support plan");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("failed to look up AWS Health service codes"),
        "{msg}"
    );
    assert!(msg.contains(DENIED), "{msg}");
}
