//! EKS and STS calls against a local AWS Emulator.
//!
//! Ignored by default. `make test-emulator` starts the emulator, seeds the
//! subnets and IAM roles below, and runs these with `--ignored`.

use std::collections::HashMap;
use std::time::Duration;

use aws_sdk_eks::Client;
use aws_sdk_eks::types::{ClusterStatus, NodegroupStatus, VpcConfigRequest};
use kuo::aws::AwsClients;
use kuo::crd::UpgradeMode;
use kuo::eks::client::EksClient;
use kuo::eks::{addon, insights, nodegroup, upgrade};

const IGNORE: &str = "needs AWS Emulator, run `make test-emulator`";
const REGION: &str = "us-east-1";
const EMULATOR_ACCOUNT: &str = "000000000000";
const ADDON: &str = "vpc-cni";

struct Seed {
    subnets: Vec<String>,
    cluster_role_arn: String,
    node_role_arn: String,
    operator_role_arn: String,
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set. {IGNORE}"))
}

fn seed() -> Seed {
    Seed {
        subnets: env("KUO_EMULATOR_SUBNETS")
            .split(',')
            .map(str::to_string)
            .collect(),
        cluster_role_arn: env("KUO_EMULATOR_CLUSTER_ROLE_ARN"),
        node_role_arn: env("KUO_EMULATOR_NODE_ROLE_ARN"),
        operator_role_arn: env("KUO_EMULATOR_OPERATOR_ROLE_ARN"),
    }
}

async fn clients() -> AwsClients {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    AwsClients::new(REGION, None).await.expect("AWS clients")
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

async fn create_cluster(client: &Client, seed: &Seed, name: &str, version: &str) {
    client
        .create_cluster()
        .name(name)
        .version(version)
        .role_arn(&seed.cluster_role_arn)
        .resources_vpc_config(
            VpcConfigRequest::builder()
                .set_subnet_ids(Some(seed.subnets.clone()))
                .build(),
        )
        .send()
        .await
        .expect("CreateCluster");
    wait_for_cluster_active(client, name).await;
}

async fn wait_for_cluster_active(client: &Client, name: &str) {
    wait_for("cluster ACTIVE", || async {
        let resp = client.describe_cluster().name(name).send().await.ok()?;
        let status = resp.cluster()?.status()?;
        (*status == ClusterStatus::Active).then_some(())
    })
    .await;
}

async fn create_nodegroup(client: &Client, seed: &Seed, cluster: &str, name: &str) {
    client
        .create_nodegroup()
        .cluster_name(cluster)
        .nodegroup_name(name)
        .set_subnets(Some(seed.subnets.clone()))
        .node_role(&seed.node_role_arn)
        .send()
        .await
        .expect("CreateNodegroup");
    wait_for("node group ACTIVE", || async {
        let resp = client
            .describe_nodegroup()
            .cluster_name(cluster)
            .nodegroup_name(name)
            .send()
            .await
            .ok()?;
        let status = resp.nodegroup()?.status()?;
        (*status == NodegroupStatus::Active).then_some(())
    })
    .await;
}

/// Installs the add-on at its oldest version compatible with `k8s_version`.
async fn create_outdated_addon(client: &Client, cluster: &str, k8s_version: &str) -> String {
    let versions = addon::get_compatible_versions(client, ADDON, k8s_version)
        .await
        .expect("compatible add-on versions");
    let oldest = versions
        .last()
        .expect("at least one version")
        .version
        .clone();
    client
        .create_addon()
        .cluster_name(cluster)
        .addon_name(ADDON)
        .addon_version(&oldest)
        .send()
        .await
        .expect("CreateAddon");
    wait_for("add-on ACTIVE", || async {
        let status = addon::poll_addon_status(client, cluster, ADDON)
            .await
            .ok()?;
        (status == "ACTIVE").then_some(())
    })
    .await;
    oldest
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn verify_identity_reports_the_caller_account() {
    let identity = clients().await.verify_identity().await.expect("identity");
    assert_eq!(identity.account_id, EMULATOR_ACCOUNT);
    assert!(
        identity.arn.starts_with("arn:aws:iam::"),
        "{}",
        identity.arn
    );
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn assume_role_signs_requests_with_the_role_session() {
    let seed = seed();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let clients = AwsClients::new(REGION, Some(&seed.operator_role_arn))
        .await
        .expect("assumed-role clients");

    let identity = clients.verify_identity().await.expect("identity");

    let role_name = seed.operator_role_arn.rsplit('/').next().unwrap();
    assert_eq!(identity.account_id, EMULATOR_ACCOUNT);
    assert_eq!(
        identity.arn,
        format!("arn:aws:sts::{EMULATOR_ACCOUNT}:assumed-role/{role_name}/kuo-operator")
    );
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn describe_cluster_reads_version_endpoint_and_ca() {
    let seed = seed();
    let clients = clients().await;
    create_cluster(&clients.eks, &seed, "describe", "1.32").await;
    let eks = EksClient::new(clients.eks.clone(), clients.region.clone());

    let info = eks
        .describe_cluster("describe")
        .await
        .expect("describe")
        .expect("cluster");

    assert_eq!(info.name, "describe");
    assert_eq!(info.version, "1.32");
    assert_eq!(info.region, REGION);
    assert!(info.endpoint.is_some_and(|e| e.starts_with("https://")));
    assert!(info.ca_data.is_some());
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn describe_cluster_fails_for_a_missing_cluster() {
    let clients = clients().await;
    let eks = EksClient::new(clients.eks.clone(), clients.region.clone());

    assert!(eks.describe_cluster("missing").await.is_err());
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn describe_cluster_versions_reads_support_lifecycle() {
    let clients = clients().await;
    let eks = EksClient::new(clients.eks.clone(), clients.region.clone());

    let lifecycle = eks
        .describe_cluster_versions(&["1.32", "1.33"])
        .await
        .expect("cluster versions");

    let versions: Vec<&str> = lifecycle.iter().map(|v| v.version.as_str()).collect();
    assert!(
        versions.contains(&"1.32") && versions.contains(&"1.33"),
        "{versions:?}"
    );
    for v in &lifecycle {
        assert!(!v.status.is_empty(), "{v:?}");
        assert!(v.end_of_standard_support.is_some(), "{v:?}");
    }
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn control_plane_update_reaches_the_target_version() {
    let seed = seed();
    let clients = clients().await;
    create_cluster(&clients.eks, &seed, "control-plane", "1.32").await;
    let eks = EksClient::new(clients.eks.clone(), clients.region.clone());

    let update_id = eks
        .update_cluster_version("control-plane", "1.33")
        .await
        .expect("update cluster version");
    let status = wait_for("control plane update", || async {
        let status = eks
            .check_update_status("control-plane", &update_id)
            .await
            .ok()?;
        (status != "InProgress").then_some(status)
    })
    .await;

    assert_eq!(status, "Successful");
    let info = eks
        .describe_cluster("control-plane")
        .await
        .expect("describe")
        .expect("cluster");
    assert_eq!(info.version, "1.33");
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn nodegroup_update_follows_the_control_plane() {
    let seed = seed();
    let clients = clients().await;
    let client = &clients.eks;
    create_cluster(client, &seed, "nodegroup", "1.32").await;
    create_nodegroup(client, &seed, "nodegroup", "ng-app").await;
    client
        .update_cluster_version()
        .name("nodegroup")
        .version("1.33")
        .send()
        .await
        .expect("UpdateClusterVersion");
    wait_for_cluster_active(client, "nodegroup").await;

    let plan = nodegroup::plan_nodegroup_upgrades(client, "nodegroup", "1.33")
        .await
        .expect("plan");
    assert_eq!(plan.upgrade_count(), 1);
    assert_eq!(plan.upgrades[0].name, "ng-app");
    assert_eq!(plan.upgrades[0].current_version(), "1.32");

    let update_id = nodegroup::update_nodegroup_version(client, "nodegroup", "ng-app", "1.33")
        .await
        .expect("update node group");
    let status = wait_for("node group update", || async {
        let status = nodegroup::poll_nodegroup_update(client, "nodegroup", "ng-app", &update_id)
            .await
            .ok()?;
        (status != "InProgress").then_some(status)
    })
    .await;
    assert_eq!(status, "Successful");

    let ng = nodegroup::describe_nodegroup(client, "nodegroup", "ng-app")
        .await
        .expect("describe")
        .expect("node group");
    assert_eq!(ng.current_version(), "1.33");
    let replan = nodegroup::plan_nodegroup_upgrades(client, "nodegroup", "1.33")
        .await
        .expect("plan");
    assert_eq!(replan.upgrade_count(), 0);
    assert_eq!(replan.skipped_count(), 1);
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn addon_upgrade_moves_to_the_default_compatible_version() {
    let seed = seed();
    let clients = clients().await;
    let client = &clients.eks;
    create_cluster(client, &seed, "addon", "1.32").await;
    let installed = create_outdated_addon(client, "addon", "1.32").await;

    let versions = addon::get_compatible_versions(client, ADDON, "1.32")
        .await
        .expect("versions");
    let default = versions
        .iter()
        .find(|v| v.default_version)
        .expect("a default version")
        .version
        .clone();
    assert_ne!(installed, default, "the seeded add-on must be outdated");
    assert_eq!(
        addon::get_latest_compatible_version(client, ADDON, "1.32")
            .await
            .expect("latest"),
        Some(default.clone())
    );

    let plan = addon::plan_addon_upgrades(client, "addon", "1.32", &HashMap::new())
        .await
        .expect("plan");
    assert_eq!(plan.upgrade_count(), 1);
    let (info, target) = &plan.upgrades[0];
    assert_eq!(info.name, ADDON);
    assert_eq!(info.current_version, installed);
    assert_eq!(*target, default);

    addon::update_addon(client, "addon", ADDON, target)
        .await
        .expect("update add-on");
    wait_for("add-on ACTIVE", || async {
        let status = addon::poll_addon_status(client, "addon", ADDON)
            .await
            .ok()?;
        (status == "ACTIVE").then_some(())
    })
    .await;
    let after = addon::describe_addon(client, "addon", ADDON)
        .await
        .expect("describe")
        .expect("add-on");
    assert_eq!(after.current_version, default);
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn addon_plan_honors_a_pinned_version() {
    let seed = seed();
    let clients = clients().await;
    let client = &clients.eks;
    create_cluster(client, &seed, "addon-pinned", "1.32").await;
    let installed = create_outdated_addon(client, "addon-pinned", "1.32").await;

    let pinned = HashMap::from([(ADDON.to_string(), installed)]);
    let plan = addon::plan_addon_upgrades(client, "addon-pinned", "1.32", &pinned)
        .await
        .expect("plan");

    assert_eq!(plan.upgrade_count(), 0);
    assert_eq!(plan.skipped_count(), 1);
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn insights_summary_matches_the_listed_insights() {
    let seed = seed();
    let clients = clients().await;
    let client = &clients.eks;
    create_cluster(client, &seed, "insights", "1.32").await;

    for category in ["UPGRADE_READINESS", "ROLLBACK_READINESS"] {
        let listed = client
            .list_insights()
            .cluster_name("insights")
            .filter(
                aws_sdk_eks::types::InsightsFilter::builder()
                    .categories(aws_sdk_eks::types::Category::from(category))
                    .build(),
            )
            .send()
            .await
            .expect("ListInsights");
        let count = listed.insights().len();

        let (ready, summary) = insights::check_insights_readiness(client, "insights", category)
            .await
            .expect("readiness");

        assert_eq!(summary.total_findings, count, "{category}");
        assert_eq!(
            summary.critical_count
                + summary.warning_count
                + summary.passing_count
                + summary.info_count,
            count,
            "{category}"
        );
        assert_eq!(ready, summary.critical_count == 0, "{category}");
        for finding in &summary.findings {
            assert_eq!(finding.category, category);
        }
    }
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn forward_plan_covers_control_plane_addons_and_nodegroups() {
    let seed = seed();
    let clients = clients().await;
    let client = &clients.eks;
    create_cluster(client, &seed, "plan-forward", "1.32").await;
    create_nodegroup(client, &seed, "plan-forward", "ng-system").await;
    create_outdated_addon(client, "plan-forward", "1.32").await;
    let eks = EksClient::new(client.clone(), clients.region.clone());

    let plan = upgrade::create_upgrade_plan(
        &eks,
        "plan-forward",
        "1.34",
        &HashMap::new(),
        UpgradeMode::Forward,
    )
    .await
    .expect("plan");

    assert_eq!(plan.current_version, "1.32");
    assert_eq!(plan.upgrade_path, ["1.33", "1.34"]);
    assert_eq!(plan.nodegroup_upgrades.len(), 1);
    assert_eq!(plan.nodegroup_upgrades[0].name, "ng-system");
    assert_eq!(plan.addon_upgrades.len(), 1);
    let latest = addon::get_latest_compatible_version(client, ADDON, "1.34")
        .await
        .expect("latest");
    assert_eq!(Some(plan.addon_upgrades[0].1.clone()), latest);
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make test-emulator`"]
async fn rollback_plan_steps_back_one_minor_without_upgrading_addons() {
    let seed = seed();
    let clients = clients().await;
    let client = &clients.eks;
    create_cluster(client, &seed, "plan-rollback", "1.33").await;
    create_nodegroup(client, &seed, "plan-rollback", "ng-system").await;
    create_outdated_addon(client, "plan-rollback", "1.33").await;
    let eks = EksClient::new(client.clone(), clients.region.clone());

    let plan = upgrade::create_upgrade_plan(
        &eks,
        "plan-rollback",
        "1.32",
        &HashMap::new(),
        UpgradeMode::Rollback,
    )
    .await
    .expect("plan");

    assert_eq!(plan.current_version, "1.33");
    assert_eq!(plan.upgrade_path, ["1.32"]);
    assert_eq!(plan.nodegroup_upgrades.len(), 1);
    for (info, target) in &plan.addon_upgrades {
        assert_ne!(&info.current_version, target);
    }
}
