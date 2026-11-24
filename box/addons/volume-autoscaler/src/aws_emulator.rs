//! Exercises the EC2 root volume growth path against AWS Emulator through the
//! real AWS SDK clients. Every test is ignored by default, `make emulator-test`
//! starts the emulator and runs them.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aws_sdk_ec2::types::{
    BlockDeviceMapping, EbsBlockDevice, IamInstanceProfileSpecification, InstanceStateName,
    InstanceType, ResourceType, Tag, TagSpecification, VolumeType,
};
use aws_sdk_ssm::types::{InstanceInformationStringFilter, PingStatus};

use crate::autoscaler::instance_volume::policy::tests::{by_tags, iva};
use crate::autoscaler::instance_volume::resizer::tests::{FakeAutoscalers, FakeSsm, Rec, config};
use crate::autoscaler::instance_volume::resizer::{Options, Resizer};
use crate::awsx::{Clients, ModifySpec};
use crate::scripts;

const AMI: &str =
    "resolve:ssm:/aws/service/ami-amazon-linux-latest/al2023-ami-kernel-default-x86_64";
const ROOT_DEVICE: &str = "/dev/xvda";
const ROOT_GIB: i32 = 20;
const SSM_ROLE_POLICY: &str = "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore";
const EC2_TRUST_POLICY: &str = r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"ec2.amazonaws.com"},"Action":"sts:AssumeRole"}]}"#;

fn region() -> String {
    std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".into())
}

/// Refuses to run unless the SDK endpoint is a loopback emulator, so these
/// tests can never launch instances in a real account.
fn require_emulator() {
    let endpoint = std::env::var("AWS_ENDPOINT_URL").unwrap_or_default();
    assert!(
        endpoint.starts_with("https://127.0.0.1:") || endpoint.starts_with("https://localhost:"),
        "AWS_ENDPOINT_URL={endpoint:?} is not a local AWS Emulator, run `make emulator-test`"
    );
}

fn unique(prefix: &str) -> String {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{prefix}-{nanos}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

async fn sdk_config() -> aws_config::SdkConfig {
    require_emulator();
    aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region()))
        .load()
        .await
}

async fn clients() -> Clients {
    require_emulator();
    let mut clients = Clients::new(&region()).await;
    clients.poll_interval = Duration::from_millis(100);
    clients
}

async fn launch(
    ec2: &aws_sdk_ec2::Client,
    tags: &[(&str, &str)],
    instance_profile: Option<&str>,
) -> String {
    let mut tag_spec = TagSpecification::builder().resource_type(ResourceType::Instance);
    for (k, v) in tags {
        tag_spec = tag_spec.tags(Tag::builder().key(*k).value(*v).build());
    }
    let out = ec2
        .run_instances()
        .image_id(AMI)
        .instance_type(InstanceType::M6iLarge)
        .min_count(1)
        .max_count(1)
        .block_device_mappings(
            BlockDeviceMapping::builder()
                .device_name(ROOT_DEVICE)
                .ebs(
                    EbsBlockDevice::builder()
                        .volume_size(ROOT_GIB)
                        .volume_type(VolumeType::Gp3)
                        .build(),
                )
                .build(),
        )
        .tag_specifications(tag_spec.build())
        .set_iam_instance_profile(instance_profile.map(|name| {
            IamInstanceProfileSpecification::builder()
                .name(name)
                .build()
        }))
        .send()
        .await
        .expect("run instances");
    let id = out.instances()[0]
        .instance_id()
        .expect("instance id")
        .to_string();
    wait_running(ec2, &id).await;
    id
}

/// `RunInstances` answers `pending`, and discovery only sees running instances.
async fn wait_running(ec2: &aws_sdk_ec2::Client, instance_id: &str) {
    for _ in 0..60 {
        let out = ec2
            .describe_instances()
            .instance_ids(instance_id)
            .send()
            .await
            .expect("describe instances");
        let state = out
            .reservations()
            .iter()
            .flat_map(aws_sdk_ec2::types::Reservation::instances)
            .find_map(|i| i.state().and_then(|s| s.name()).cloned());
        if state == Some(InstanceStateName::Running) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("{instance_id} never reached running");
}

async fn ssm_instance_profile(iam: &aws_sdk_iam::Client) -> String {
    let name = unique("volume-autoscaler-ssm");
    iam.create_role()
        .role_name(&name)
        .assume_role_policy_document(EC2_TRUST_POLICY)
        .send()
        .await
        .expect("create role");
    iam.attach_role_policy()
        .role_name(&name)
        .policy_arn(SSM_ROLE_POLICY)
        .send()
        .await
        .expect("attach policy");
    iam.create_instance_profile()
        .instance_profile_name(&name)
        .send()
        .await
        .expect("create instance profile");
    iam.add_role_to_instance_profile()
        .instance_profile_name(&name)
        .role_name(&name)
        .send()
        .await
        .expect("add role to instance profile");
    name
}

async fn wait_ssm_online(ssm: &aws_sdk_ssm::Client, instance_id: &str) {
    for _ in 0..60 {
        let out = ssm
            .describe_instance_information()
            .filters(
                InstanceInformationStringFilter::builder()
                    .key("InstanceIds")
                    .values(instance_id)
                    .build()
                    .expect("filter"),
            )
            .send()
            .await
            .expect("describe instance information");
        if out
            .instance_information_list()
            .iter()
            .any(|i| i.ping_status() == Some(&PingStatus::Online))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("{instance_id} never came online in SSM");
}

async fn root_volume(clients: &Clients, instance_id: &str) -> (String, i32) {
    let inst = clients
        .describe_target_instances()
        .await
        .expect("describe target instances")
        .into_iter()
        .find(|i| i.id == instance_id)
        .expect("launched instance discovered");
    (inst.root_volume_id, inst.root_volume_size_gib)
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make emulator-test`"]
async fn root_volume_grows_through_modification_states() {
    let cfg = sdk_config().await;
    let suite = unique("grow");
    let id = launch(
        &aws_sdk_ec2::Client::new(&cfg),
        &[("Name", "emulator-node"), ("Suite", &suite)],
        None,
    )
    .await;
    let clients = clients().await;

    let inst = clients
        .describe_target_instances()
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.id == id)
        .unwrap();
    assert_eq!(inst.name, "emulator-node");
    assert_eq!(inst.tags["Suite"], suite);
    assert_eq!(inst.root_device_name, ROOT_DEVICE);
    assert!(inst.root_volume_id.starts_with("vol-"));
    assert_eq!(inst.root_volume_size_gib, ROOT_GIB);
    let volume = inst.root_volume_id;

    assert_eq!(
        clients.describe_last_modification(&volume).await.unwrap(),
        None,
        "InvalidVolumeModification.NotFound reads as never modified"
    );

    clients
        .modify_volume(
            &volume,
            ModifySpec {
                size_gib: 25,
                ..ModifySpec::default()
            },
        )
        .await
        .unwrap();
    let m = clients
        .describe_last_modification(&volume)
        .await
        .unwrap()
        .expect("modification recorded");
    assert_eq!(m.state, "modifying");
    assert_eq!(m.target_gib, 25);
    assert!(m.start_time.is_some());

    let err = clients
        .modify_volume(
            &volume,
            ModifySpec {
                size_gib: 30,
                ..ModifySpec::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("IncorrectModificationState"));
    assert!(
        err.message
            .contains(&format!("modify volume {volume} to 30 GiB"))
    );

    clients
        .wait_for_modification(&volume, Duration::from_secs(10))
        .await
        .unwrap();
    let state = clients
        .describe_last_modification(&volume)
        .await
        .unwrap()
        .unwrap()
        .state;
    assert!(
        matches!(state.as_str(), "optimizing" | "completed"),
        "{state}"
    );

    assert_eq!(root_volume(&clients, &id).await, (volume, 25));
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make emulator-test`"]
async fn throughput_and_iops_ride_along_with_the_resize() {
    let cfg = sdk_config().await;
    let id = launch(&aws_sdk_ec2::Client::new(&cfg), &[], None).await;
    let clients = clients().await;
    let (volume, _) = root_volume(&clients, &id).await;

    clients
        .modify_volume(
            &volume,
            ModifySpec {
                size_gib: 30,
                throughput_mibps: 250,
                iops: 4000,
            },
        )
        .await
        .unwrap();
    clients
        .wait_for_modification(&volume, Duration::from_secs(10))
        .await
        .unwrap();

    let attached = clients
        .describe_attached_volumes(std::slice::from_ref(&id))
        .await
        .unwrap();
    let vols = &attached[&id];
    assert_eq!(vols.len(), 1);
    assert_eq!(vols[0].id, volume);
    assert_eq!(vols[0].device, ROOT_DEVICE);
    assert_eq!(vols[0].kind, "gp3");
    assert_eq!(vols[0].size_gib, 30);
    assert_eq!(vols[0].throughput_mibps, 250);
    assert_eq!(vols[0].iops, 4000);

    let caps = clients
        .describe_instance_type_ebs_caps(&["m6i.large".to_string()])
        .await
        .unwrap();
    let caps = caps["m6i.large"];
    assert!(caps.baseline_mbps > 0.0);
    assert!(caps.maximum_mbps >= caps.baseline_mbps);
}

#[tokio::test]
#[ignore = "needs AWS Emulator, run `make emulator-test`"]
async fn run_script_reports_guest_output_and_failures() {
    let cfg = sdk_config().await;
    let ec2 = aws_sdk_ec2::Client::new(&cfg);
    let profile = ssm_instance_profile(&aws_sdk_iam::Client::new(&cfg)).await;
    let managed = launch(&ec2, &[], Some(&profile)).await;
    let unmanaged = launch(&ec2, &[], None).await;
    wait_ssm_online(&aws_sdk_ssm::Client::new(&cfg), &managed).await;
    let clients = clients().await;
    let timeout = Duration::from_secs(30);

    let ok = clients
        .run_script(&managed, "echo 87", timeout)
        .await
        .unwrap();
    assert_eq!(ok.status, "Success");
    assert_eq!(ok.exit_code, 0);
    assert_eq!(ok.stdout, "87\n");

    let err = clients
        .run_script(&managed, "no-such-command", timeout)
        .await
        .unwrap_err();
    assert!(
        err.message.contains("ended with status Failed (exit 127)"),
        "{}",
        err.message
    );

    let err = clients
        .run_script(&unmanaged, "echo 1", timeout)
        .await
        .unwrap_err();
    assert!(
        err.message
            .starts_with(&format!("send command to {unmanaged}:")),
        "{}",
        err.message
    );
}

/// The emulated guest shell has no `df`, `growpart`, or `xfs_growfs`, so the
/// filesystem side is scripted while EC2 is the emulator.
#[tokio::test]
#[ignore = "needs AWS Emulator, run `make emulator-test`"]
async fn reconcile_grows_a_tagged_instance_end_to_end() {
    let cfg = sdk_config().await;
    let suite = unique("reconcile");
    let id = launch(
        &aws_sdk_ec2::Client::new(&cfg),
        &[("Name", "emulator-node"), ("Suite", &suite)],
        None,
    )
    .await;
    let clients = Arc::new(clients().await);
    let ssm = Arc::new(FakeSsm {
        measurements: Mutex::new(vec![Ok("90\n".into()), Ok("45\n".into())]),
        ..FakeSsm::default()
    });
    let rec = Arc::new(Rec::default());
    let autoscalers = Arc::new(FakeAutoscalers {
        ivas: vec![iva("emulator", by_tags(&[("Suite", &suite)]))],
        ..FakeAutoscalers::default()
    });
    let resizer = Arc::new(Resizer::new(
        Arc::new(config()),
        clients.clone(),
        ssm.clone(),
        rec.clone(),
        Options {
            autoscalers: Some(autoscalers.clone()),
            ..Options::default()
        },
    ));

    assert!(resizer.reconcile().await.unwrap() >= 1);

    assert_eq!(
        rec.resizes.lock().unwrap().clone(),
        vec![(true, "emulator".to_string())]
    );
    assert_eq!(
        ssm.scripts.lock().unwrap().clone(),
        vec![
            scripts::MEASURE_ROOT_FS.to_string(),
            scripts::RESIZE_ROOT_FS.to_string(),
            scripts::MEASURE_ROOT_FS.to_string(),
        ]
    );
    let (volume, size) = root_volume(&clients, &id).await;
    assert!(size > ROOT_GIB, "grew past {ROOT_GIB} GiB: {size}");
    assert_eq!(
        rec.sizes.lock().unwrap().clone(),
        vec![(id.clone(), ROOT_GIB), (id.clone(), size)]
    );
    let m = clients
        .describe_last_modification(&volume)
        .await
        .unwrap()
        .expect("modification recorded");
    assert_eq!(m.target_gib, size);
    let patched = autoscalers.patched.lock().unwrap().clone();
    assert_eq!(
        patched.last().map(|(name, _)| name.as_str()),
        Some("emulator")
    );
}
