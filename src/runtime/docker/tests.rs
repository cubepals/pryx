// SPDX-License-Identifier: FSL-1.1-ALv2
//! Docker construction and host process ownership regressions. Real engine behavior belongs
//! to the existing integration suite; this module has no simulated engine status codes.
use super::*;

pub(super) fn spec() -> ContainerSpec {
    ContainerSpec {
        packages: Vec::new(),
        agent: None,
        cgroup_parent: None,
        name: "blockly-test-w".into(),
        image: "alpine:3.22".into(),
        entrypoint: None,
        workdir: None,
        egress: None,
        env: Vec::new(),
        labels: BTreeMap::new(),
        user: "1000:1000".into(),
        memory_bytes: 256 * 1024 * 1024,
        allow_unenforced_memory: None,
        require_cpu_limit: false,
        require_pids_limit: false,
        nano_cpus: None,
        cpu_shares: 1024,
        pids_limit: 4096,
        read_only_rootfs: true,
        tmp_size_mb: 64,
        data_dir: "/var/lib/pryxd/workloads/w/data".into(),
        mount_path: "/data".into(),
        ports: Vec::new(),
        stop_signal: "SIGTERM".into(),
        stop_timeout_secs: 90,
        network: "blockly-workloads".into(),
        internal_network: false,
        log_max_size_mb: 10,
        log_max_files: 3,
        oom_score_adj: 500,
    }
}

#[test]
fn explicit_entrypoint_does_not_append_the_images_default_command() {
    let mut spec = spec();
    assert_eq!(body_of(&spec).cmd, None);
    spec.entrypoint = Some(vec!["echo".into(), "exact".into()]);
    assert_eq!(body_of(&spec).entrypoint, spec.entrypoint);
    assert_eq!(body_of(&spec).cmd, Some(Vec::new()));
}

#[test]
fn docker_is_never_asked_to_restart_a_workload() {
    let host = body_of(&spec()).host_config.unwrap();
    assert_eq!(
        host.restart_policy,
        Some(DockerRestartPolicy { name: Some(RestartPolicyNameEnum::NO), maximum_retry_count: None })
    );
}

#[test]
fn only_a_reference_without_tag_or_digest_is_pulled_as_latest() {
    for (image, tag) in [
        ("repo", Some("latest")),
        ("a/b/c", Some("latest")),
        ("reg:5000/repo", Some("latest")),
        ("repo:1", None),
        ("reg:5000/repo:1", None),
        ("repo@sha256:abc", None),
        ("reg:5000/repo@sha256:abc", None),
    ] {
        assert_eq!(pull_tag(image), tag, "{image}");
    }
}

#[test]
fn the_tree_walk_never_starts_from_0_or_1() {
    let children = HashMap::from([(0, vec![1, 2]), (1, vec![100, 200]), (100, vec![101]), (101, vec![102])]);
    assert!(termination::descendants(0, &children).is_empty());
    assert!(termination::descendants(1, &children).is_empty());
    assert!(termination::descendants(-1, &children).is_empty());
    assert_eq!(termination::descendants(100, &children), vec![100, 101, 102]);
    assert_eq!(termination::descendants(200, &children), vec![200], "a leaf is its own tree");
}

#[test]
fn killing_from_0_or_1_signals_nothing() {
    // An id no cgroup names, so even a walk that did start could kill nothing on this host.
    let container = "0".repeat(64);
    assert_eq!(termination::kill_exec_tree(0, &container), 0);
    assert_eq!(termination::kill_exec_tree(1, &container), 0);
    assert_eq!(termination::kill_exec_tree(-1, &container), 0);
}
