//! RDS and Performance Insights calls against a local AWS Emulator.
//!
//! Ignored by default. `make test-emulator` starts the emulator and runs these
//! with `--ignored`. Every test owns one Aurora cluster and narrows discovery to
//! it with an identifier filter, so the tests share one emulator account.

use std::sync::Arc;
use std::time::Duration;

use adie::aws::collector::{AwsPiCollector, PiCollector, run_collection_cycle};
use adie::aws::discovery::{
    AwsRdsDiscoverer, InstanceChannel, RdsDiscoverer, RdsInstanceInfo, run_discovery_cycle,
};
use adie::config::{CollectionConfig, DiscoveryConfig, FilterConfig, RetryConfig, TagFilter};
use adie::observability::metrics::Metrics;
use adie::types::AuroraInstance;
use aws_sdk_rds::Client as RdsClient;
use aws_sdk_rds::types::Tag;
use tokio::sync::watch;

const IGNORE: &str = "needs AWS Emulator, run `make test-emulator`";
const REGION: &str = "us-east-1";
const MYSQL: &str = "aurora-mysql";
const POSTGRESQL: &str = "aurora-postgresql";
const PAGINATION_REGION: &str = "us-west-2";
const RDS_MIN_PAGE_SIZE: i32 = 20;

struct Clients {
    rds: RdsClient,
    pi: aws_sdk_pi::Client,
}

async fn clients() -> Clients {
    clients_in(REGION).await
}

async fn clients_in(region: &'static str) -> Clients {
    assert!(
        std::env::var("AWS_ENDPOINT_URL").is_ok(),
        "AWS_ENDPOINT_URL is not set. {IGNORE}"
    );
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region))
        .load()
        .await;
    Clients {
        rds: RdsClient::new(&config),
        pi: aws_sdk_pi::Client::new(&config),
    }
}

/// Polls `check` every 500 ms until it returns `Some`, for at most 60 s.
async fn wait_for<T, F, Fut>(what: &str, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    for _ in 0..120 {
        if let Some(value) = check().await {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn create_cluster(rds: &RdsClient, name: &str, engine: &str) {
    rds.create_db_cluster()
        .db_cluster_identifier(name)
        .engine(engine)
        .master_username("dbadmin")
        .manage_master_user_password(true)
        .send()
        .await
        .expect("CreateDBCluster");
}

struct InstanceSpec<'a> {
    name: &'a str,
    cluster: &'a str,
    engine: &'a str,
    class: &'a str,
    performance_insights: bool,
    tags: &'a [(&'a str, &'a str)],
}

impl<'a> InstanceSpec<'a> {
    const fn new(name: &'a str, cluster: &'a str) -> Self {
        Self {
            name,
            cluster,
            engine: MYSQL,
            class: "db.r6g.large",
            performance_insights: true,
            tags: &[],
        }
    }
}

async fn create_instance(rds: &RdsClient, spec: &InstanceSpec<'_>) {
    let tags = spec
        .tags
        .iter()
        .map(|(key, value)| Tag::builder().key(*key).value(*value).build())
        .collect();
    rds.create_db_instance()
        .db_instance_identifier(spec.name)
        .db_cluster_identifier(spec.cluster)
        .engine(spec.engine)
        .db_instance_class(spec.class)
        .enable_performance_insights(spec.performance_insights)
        .set_tags(Some(tags))
        .send()
        .await
        .expect("CreateDBInstance");
}

fn discovery_config(prefix: &str) -> DiscoveryConfig {
    DiscoveryConfig {
        include: FilterConfig {
            identifier: vec![format!("^{prefix}-")],
            tags: Vec::new(),
        },
        ..DiscoveryConfig::default()
    }
}

fn channel() -> InstanceChannel {
    watch::channel(Arc::new(Vec::<AuroraInstance>::new())).0
}

fn identifiers(state: &InstanceChannel) -> Vec<String> {
    let mut names: Vec<String> = state
        .borrow()
        .iter()
        .map(|instance| instance.db_instance_identifier.clone())
        .collect();
    names.sort();
    names
}

async fn describe(discoverer: &AwsRdsDiscoverer, name: &str) -> RdsInstanceInfo {
    discoverer
        .describe_instances()
        .await
        .expect("DescribeDBInstances")
        .into_iter()
        .find(|instance| instance.db_instance_identifier == name)
        .unwrap_or_else(|| panic!("{name} is missing from DescribeDBInstances"))
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn discovery_maps_every_instance_field() {
    let clients = clients().await;
    create_cluster(&clients.rds, "fields", MYSQL).await;
    create_instance(
        &clients.rds,
        &InstanceSpec {
            tags: &[("Team", "payments"), ("Environment", "dev")],
            ..InstanceSpec::new("fields-writer", "fields")
        },
    )
    .await;
    create_instance(
        &clients.rds,
        &InstanceSpec {
            class: "db.t4g.medium",
            performance_insights: false,
            ..InstanceSpec::new("fields-reader", "fields")
        },
    )
    .await;

    let discoverer = AwsRdsDiscoverer::new(clients.rds);
    let writer = describe(&discoverer, "fields-writer").await;
    assert!(writer.dbi_resource_id.starts_with("db-"));
    assert_eq!(writer.engine, MYSQL);
    assert_eq!(writer.db_cluster_identifier, "fields");
    assert_eq!(writer.db_instance_class, "db.r6g.large");
    assert!(writer.performance_insights_enabled);
    let mut tags = writer.tags;
    tags.sort();
    assert_eq!(
        tags,
        vec![
            ("Environment".to_string(), "dev".to_string()),
            ("Team".to_string(), "payments".to_string()),
        ]
    );

    let reader = describe(&discoverer, "fields-reader").await;
    assert_eq!(reader.db_instance_class, "db.t4g.medium");
    assert!(!reader.performance_insights_enabled);
    assert_eq!(reader.tags, []);
    assert_ne!(reader.dbi_resource_id, writer.dbi_resource_id);
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn discovery_cycle_applies_every_filter() {
    let clients = clients().await;
    create_cluster(&clients.rds, "filter", MYSQL).await;
    create_cluster(&clients.rds, "filter-pg", POSTGRESQL).await;
    let payments = [("Team", "payments")];
    create_instance(
        &clients.rds,
        &InstanceSpec {
            tags: &payments,
            ..InstanceSpec::new("filter-writer", "filter")
        },
    )
    .await;
    create_instance(
        &clients.rds,
        &InstanceSpec {
            tags: &payments,
            performance_insights: false,
            ..InstanceSpec::new("filter-no-insights", "filter")
        },
    )
    .await;
    create_instance(
        &clients.rds,
        &InstanceSpec {
            tags: &payments,
            ..InstanceSpec::new("filter-excluded", "filter")
        },
    )
    .await;
    create_instance(
        &clients.rds,
        &InstanceSpec {
            tags: &[("Team", "search")],
            ..InstanceSpec::new("filter-other-team", "filter")
        },
    )
    .await;
    create_instance(
        &clients.rds,
        &InstanceSpec {
            engine: POSTGRESQL,
            tags: &payments,
            ..InstanceSpec::new("filter-postgres", "filter-pg")
        },
    )
    .await;

    let discoverer = AwsRdsDiscoverer::new(clients.rds);
    let mut config = discovery_config("filter");
    config.include.tags = vec![TagFilter {
        key: "Team".to_string(),
        value: "payments".to_string(),
    }];
    config.exclude.identifier = vec!["excluded$".to_string()];
    config.exported_tags = vec!["Team".to_string(), "Missing".to_string()];

    let state = channel();
    let result = run_discovery_cycle(&discoverer, &config, &state)
        .await
        .expect("discovery cycle");
    assert_eq!(result.total, 1);
    assert_eq!(result.added, 1);
    assert_eq!(result.removed_instances, []);
    let published = Arc::clone(&state.borrow());
    assert_eq!(published[0].db_instance_identifier, "filter-writer");
    assert_eq!(published[0].vcpu, 2);
    assert_eq!(
        published[0].exported_tags,
        [
            ("Team".to_string(), "payments".to_string()),
            ("Missing".to_string(), String::new()),
        ]
    );

    config.engines.push(POSTGRESQL.to_string());
    config.require_pi_enabled = false;
    let result = run_discovery_cycle(&discoverer, &config, &state)
        .await
        .expect("discovery cycle");
    assert_eq!(result.added, 2);
    assert_eq!(
        identifiers(&state),
        ["filter-no-insights", "filter-postgres", "filter-writer"]
    );
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn discovery_cycle_reports_deleted_instances() {
    let clients = clients().await;
    create_cluster(&clients.rds, "removal", MYSQL).await;
    create_instance(&clients.rds, &InstanceSpec::new("removal-keep", "removal")).await;
    create_instance(&clients.rds, &InstanceSpec::new("removal-drop", "removal")).await;

    let discoverer = AwsRdsDiscoverer::new(clients.rds.clone());
    let config = discovery_config("removal");
    let state = channel();
    let result = run_discovery_cycle(&discoverer, &config, &state)
        .await
        .expect("discovery cycle");
    assert_eq!(result.total, 2);

    clients
        .rds
        .delete_db_instance()
        .db_instance_identifier("removal-drop")
        .skip_final_snapshot(true)
        .send()
        .await
        .expect("DeleteDBInstance");
    let result = wait_for("removal-drop to leave discovery", || async {
        let result = run_discovery_cycle(&discoverer, &config, &state)
            .await
            .expect("discovery cycle");
        (!result.removed_instances.is_empty()).then_some(result)
    })
    .await;

    assert_eq!(result.total, 1);
    assert_eq!(result.added, 0);
    assert_eq!(result.removed_instances.len(), 1);
    assert_eq!(
        result.removed_instances[0].db_instance_identifier,
        "removal-drop"
    );
    assert_eq!(identifiers(&state), ["removal-keep"]);
}

/// Runs in its own region because the emulator allows 40 instances per region.
#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn discovery_follows_the_pagination_marker() {
    let rds = clients_in(PAGINATION_REGION).await.rds;
    create_cluster(&rds, "pages", MYSQL).await;
    let count = usize::try_from(RDS_MIN_PAGE_SIZE).expect("page size") + 5;
    for index in 0..count {
        let name = format!("pages-{index:02}");
        create_instance(
            &rds,
            &InstanceSpec {
                class: "db.t4g.medium",
                ..InstanceSpec::new(&name, "pages")
            },
        )
        .await;
    }

    let discoverer = AwsRdsDiscoverer::new(rds).with_page_size(RDS_MIN_PAGE_SIZE);
    let state = channel();
    let result = run_discovery_cycle(&discoverer, &discovery_config("pages"), &state)
        .await
        .expect("discovery cycle");
    assert_eq!(result.total, count);
    let names = identifiers(&state);
    assert_eq!(names.first().map(String::as_str), Some("pages-00"));
    assert_eq!(names.last().map(String::as_str), Some("pages-24"));
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn dimension_keys_are_accepted_for_every_group() {
    let clients = clients().await;
    create_cluster(&clients.rds, "keys", MYSQL).await;
    create_cluster(&clients.rds, "keys-pg", POSTGRESQL).await;
    create_instance(&clients.rds, &InstanceSpec::new("keys-mysql", "keys")).await;
    create_instance(
        &clients.rds,
        &InstanceSpec {
            engine: POSTGRESQL,
            ..InstanceSpec::new("keys-postgres", "keys-pg")
        },
    )
    .await;

    let discoverer = AwsRdsDiscoverer::new(clients.rds);
    let collector = AwsPiCollector::new(clients.pi);
    for name in ["keys-mysql", "keys-postgres"] {
        let instance = describe(&discoverer, name).await;
        for group in ["db.sql_tokenized", "db.sql", "db.user", "db.host"] {
            let keys = collector
                .describe_dimension_keys(&instance.dbi_resource_id, group, 10, 60, &instance.engine)
                .await
                .unwrap_or_else(|e| panic!("DescribeDimensionKeys {group} for {name}: {e}"));
            assert!(keys.len() <= 10);
            assert!(keys.iter().all(|key| !key.dimensions.is_empty()));
        }
    }
}

/// The emulator scaffolds grouped `GetResourceMetrics` without implementing it, so today this
/// covers the failure path. The success branch takes over once the emulator returns data.
#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn collection_cycle_settles_the_up_metric() {
    let clients = clients().await;
    create_cluster(&clients.rds, "collect", MYSQL).await;
    create_instance(
        &clients.rds,
        &InstanceSpec::new("collect-writer", "collect"),
    )
    .await;

    let discoverer = AwsRdsDiscoverer::new(clients.rds);
    let state = channel();
    run_discovery_cycle(&discoverer, &discovery_config("collect"), &state)
        .await
        .expect("discovery cycle");
    let instances = Arc::clone(&state.borrow());
    assert_eq!(instances.len(), 1);

    let config = CollectionConfig {
        retry: RetryConfig {
            max_attempts: 2,
            base_delay_ms: 10,
        },
        ..CollectionConfig::default()
    };
    let metrics = Arc::new(Metrics::new(&[]).expect("metrics"));
    let collector = Arc::new(AwsPiCollector::new(clients.pi));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
    let (collected, failed) = run_collection_cycle(
        &collector, &instances, REGION, &config, &metrics, &semaphore,
    )
    .await;
    assert_eq!(collected + failed, 1);

    let exposition = metrics.encode();
    let up = exposition
        .lines()
        .find(|line| line.starts_with("aurora_dbinsights_up{"))
        .expect("aurora_dbinsights_up series");
    assert!(up.contains(r#"instance="collect-writer""#));
    if collected == 1 {
        assert!(up.ends_with(" 1"), "{up}");
    } else {
        assert!(up.ends_with(" 0"), "{up}");
        let errors = exposition
            .lines()
            .find(|line| line.starts_with("aurora_dbinsights_pi_api_errors_total{"))
            .expect("aurora_dbinsights_pi_api_errors_total series");
        assert!(errors.contains(r#"instance="collect-writer""#));
        assert!(errors.ends_with(" 2"), "{errors}");
    }
}

/// A grouped `GetResourceMetrics` failure has to carry the service error code, which the
/// `error_kind` label is derived from.
#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn resource_metrics_errors_keep_the_service_error_code() {
    let clients = clients().await;
    create_cluster(&clients.rds, "errors", MYSQL).await;
    create_instance(&clients.rds, &InstanceSpec::new("errors-writer", "errors")).await;
    let instance = describe(&AwsRdsDiscoverer::new(clients.rds), "errors-writer").await;

    let collector = AwsPiCollector::new(clients.pi);
    let result = collector
        .get_resource_metrics_grouped(&instance.dbi_resource_id, "db.wait_event", Some(25), 60)
        .await;
    if let Err(message) = result {
        assert!(message.contains("NotImplemented"), "{message}");
    }
}
