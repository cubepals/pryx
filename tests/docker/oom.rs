// SPDX-License-Identifier: FSL-1.1-ALv2
//! The real engine memory bound and truthful OOM reporting. Lifecycle cases live in the parent.
use super::*;

#[tokio::test]
#[ignore = "needs Docker"]
async fn repeated_normal_exit_inspections_do_not_wait_for_future_events() {
    use bollard::models::{ContainerCreateBody, HostConfig};
    use bollard::query_parameters::CreateContainerOptionsBuilder;
    use futures_util::StreamExt;

    let dep = deployment();
    let _cleanup = Cleanup(dep.clone());
    let socket = DockerConfig::default().socket;
    let runtime = gvisor::engine_runtime(&socket);
    runtime.info().await.unwrap();
    let image = "alpine:3.22";
    runtime.ensure_image(image, Duration::from_secs(120)).await.unwrap();
    let engine = bollard::Docker::connect_with_unix(socket.to_str().unwrap(), 60, bollard::API_DEFAULT_VERSION)
        .unwrap()
        .negotiate_version()
        .await
        .unwrap();
    let name = format!("normal-exit-{dep}");
    let options = CreateContainerOptionsBuilder::new().name(&name).build();
    let made = engine
        .create_container(
            Some(options),
            ContainerCreateBody {
                image: Some(image.into()),
                cmd: Some(vec!["/bin/sh".into(), "-c".into(), "exit 0".into()]),
                labels: Some([(LABEL_DEPLOYMENT.to_owned(), dep)].into_iter().collect()),
                host_config: Some(HostConfig {
                    network_mode: Some("none".into()),
                    runtime: gvisor::on_gvisor().then(|| pryxd::runtime::docker::GVISOR_RUNTIME.into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    engine.start_container(&made.id, None).await.unwrap();
    let exit = engine.wait_container(&made.id, None).next().await.unwrap().unwrap();
    assert_eq!(exit.status_code, 0);
    // Four future-second history reads take at least three seconds even for one unchanged
    // normal exit. This budget covers ordinary local API work without waiting on that timer.
    let observed = tokio::time::timeout(Duration::from_secs(2), async {
        for _ in 0..4 {
            let info = serde_json::to_value(runtime.inspect(&made.id).await.unwrap().unwrap()).unwrap();
            assert_eq!(info["id"], made.id);
            assert_eq!(info["status"], serde_json::to_value(pryxd::runtime::ContainerStatus::Exited).unwrap());
            assert_eq!(info["exit_code"], 0);
            assert_eq!(info["oom_killed"], false, "normal exits never become OOM evidence");
        }
    })
    .await;
    assert!(observed.is_ok(), "normal-exit inspection waited for future event cutoffs");
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn the_memory_limit_is_hard_and_an_oom_kill_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let dep = deployment();
    let _cleanup = Cleanup(dep.clone());
    let m = daemon(dir.path(), &dep);
    m.reconcile(Records::FromDisk).await;
    // Repeated fast exits exercise Docker's gVisor inspect/event ordering. No test handshake
    // delays PID 1's exit until Docker reports OOM: a real workload has no such handshake.
    let trials = if gvisor::on_gvisor() { 10 } else { 1 };
    for trial in 0..trials {
        let name = format!("oom{trial}");
        // Linux include/uapi/linux/oom.h: prefer the allocating children as victims.
        const OOM_SCORE_ADJ_MAX: i32 = 1000;
        let mut spec = shell(
            &format!(
                "sleep 1; mkfifo /tmp/oom-input || exit 1; \
                 sh -c 'echo {OOM_SCORE_ADJ_MAX} > /proc/self/oom_score_adj || exit 1; exec head -c 400m /dev/zero' > /tmp/oom-input & producer=$!; \
                 sh -c 'echo {OOM_SCORE_ADJ_MAX} > /proc/self/oom_score_adj || exit 1; exec tail /tmp/oom-input' > /dev/null & \
                 allocator=$!; wait \"$allocator\"; status=$?; wait \"$producer\"; exit \"$status\""
            ),
            64,
        );
        spec.restart.max_retries = 0;
        m.ensure(id(&name), spec, Precondition::None, None).await.unwrap();
        m.start(id(&name), None).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            m.stats(&id(&name)).await.ok();
            let view = m.view(&id(&name)).unwrap();
            if view.exit.is_some() || Instant::now() >= deadline {
                if !view.exit.as_ref().is_some_and(|exit| exit.oom_killed) {
                    failure_evidence(view.locate.container_id.as_deref()).await;
                }
                assert!(
                    view.exit.as_ref().is_some_and(|exit| exit.oom_killed),
                    "OOM trial {trial}: {:?} {:?}",
                    view.state,
                    view.exit
                );
                assert_eq!(view.exit.unwrap().code, pryxd::runtime::exit_by(libc::SIGKILL));
                // A fresh backend has no live event cache: Docker's retained event must recover
                // the same result across daemon restart, even if inspect lost its OOM flag.
                let runtime = gvisor::engine_runtime(&DockerConfig::default().socket);
                runtime.info().await.unwrap();
                let recovered = runtime.inspect(&view.locate.container_id.unwrap()).await.unwrap().unwrap();
                assert_eq!(serde_json::to_value(recovered).unwrap()["oom_killed"], true, "fresh backend trial {trial}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        cleanup(&m, &[&name]).await;
    }
}

/// Capture only this owned container's fixed OOM fields before panic cleanup removes it.
/// Never include its environment, command, labels or arbitrary event attributes.
async fn failure_evidence(id: Option<&str>) {
    use bollard::query_parameters::EventsOptionsBuilder;
    use futures_util::StreamExt;
    use std::collections::HashMap;
    let Some(id) = id else {
        eprintln!("OOM_FAILURE_EVIDENCE missing immutable container ID");
        return;
    };
    let socket = DockerConfig::default().socket;
    let Ok(docker) = bollard::Docker::connect_with_unix(socket.to_str().unwrap(), 3, bollard::API_DEFAULT_VERSION)
    else {
        eprintln!("OOM_FAILURE_EVIDENCE engine client unavailable");
        return;
    };
    let capture = async {
        let inspect = docker.inspect_container(id, None).await?;
        let state = inspect.state.clone().unwrap_or_default();
        eprintln!(
            "OOM_FAILURE_STATE {}",
            serde_json::json!({
                "id": inspect.id, "status": state.status, "oomKilled": state.oom_killed,
                "exitCode": state.exit_code, "startedAt": state.started_at, "finishedAt": state.finished_at,
                "pid": state.pid,
            })
        );
        let host = inspect.host_config.as_ref();
        eprintln!(
            "OOM_FAILURE_LIMITS {}",
            serde_json::json!({
                "memory": host.and_then(|h| h.memory),
                "memorySwap": host.and_then(|h| h.memory_swap),
                "pidsLimit": host.and_then(|h| h.pids_limit),
                "oomKillDisable": host.and_then(|h| h.oom_kill_disable),
                "oomScoreAdj": host.and_then(|h| h.oom_score_adj),
                "runtime": host.and_then(|h| h.runtime.as_deref()).map(|runtime| match runtime {
                    "runc" | "runsc" | "crun" | "io.containerd.runc.v2" => runtime,
                    _ => "other",
                }),
            })
        );
        eprintln!("OOM_FAILURE_KERNEL {}", current_leaf_events(&docker, id, &inspect).await);
        let now = time::OffsetDateTime::now_utc();
        let since = (now - time::Duration::minutes(1)).unix_timestamp().to_string();
        let until = now.unix_timestamp().saturating_add(1).to_string();
        let filters = HashMap::from([
            ("type".to_owned(), vec!["container".to_owned()]),
            ("container".to_owned(), vec![id.to_owned()]),
            ("event".to_owned(), vec!["oom".to_owned(), "start".to_owned(), "die".to_owned()]),
        ]);
        let options = EventsOptionsBuilder::new().since(&since).until(&until).filters(&filters).build();
        let mut events = docker.events(Some(options));
        while let Some(event) = events.next().await {
            let event = event?;
            eprintln!(
                "OOM_FAILURE_EVENT {}",
                serde_json::json!({
                    "id": event.actor.and_then(|actor| actor.id), "action": event.action, "timeNano": event.time_nano,
                })
            );
        }
        match docker.version().await {
            Ok(version) => {
                let components = version
                    .components
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|component| {
                        matches!(component.name.as_str(), "Engine" | "containerd" | "runc" | "docker-init")
                    })
                    .map(|component| {
                        let version = diagnostic_version(&component.version).map(str::to_owned);
                        (component.name, version)
                    })
                    .collect::<std::collections::BTreeMap<_, _>>();
                eprintln!(
                    "OOM_FAILURE_VERSIONS {}",
                    serde_json::json!({
                        "engine": version.version.as_deref().and_then(diagnostic_version),
                        "api": version.api_version.as_deref().and_then(diagnostic_version),
                        "components": components,
                    })
                );
            }
            Err(_) => eprintln!("OOM_FAILURE_VERSIONS unavailable"),
        }
        Ok::<_, bollard::errors::Error>(())
    };
    match tokio::time::timeout(Duration::from_secs(3), capture).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => eprintln!("OOM_FAILURE_EVIDENCE engine read unavailable"),
        Err(_) => eprintln!("OOM_FAILURE_EVIDENCE capture deadline"),
    }
}

/// Only fixed public version tokens, never component details or arbitrary owner strings.
fn diagnostic_version(version: &str) -> Option<&str> {
    (!version.is_empty()
        && version.len() <= 128
        && version.bytes().all(|byte| byte.is_ascii_alphanumeric() || b".+-_".contains(&byte)))
    .then_some(version)
}

/// A still-live owned leaf only. An exited container's deleted cgroup cannot supply evidence;
/// absence is reported, never read as a zero counter or replaced with another cgroup's values.
/// File names and counter names are Linux's Documentation/admin-guide/cgroup-v2.rst ABI.
async fn current_leaf_events(
    docker: &bollard::Docker,
    id: &str,
    inspect: &bollard::models::ContainerInspectResponse,
) -> serde_json::Value {
    use std::os::unix::fs::MetadataExt;
    use std::path::{Component, Path};

    let missing = |reason| serde_json::json!({"observed": false, "reason": reason});
    if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return missing("immutable_id_unconfirmed");
    }
    let Some(state) = inspect.state.as_ref() else { return missing("no_owner_state") };
    let Some(pid) = state.pid.filter(|pid| *pid > 0) else { return missing("no_live_owner_pid") };
    if inspect.id.as_deref() != Some(id) || state.started_at.as_ref().is_none_or(String::is_empty) {
        return missing("owner_identity_unconfirmed");
    }
    let Ok(membership) = std::fs::read_to_string(format!("/proc/{pid}/cgroup")) else {
        return missing("membership_unavailable");
    };
    let Some(group) = membership.lines().find_map(|line| line.strip_prefix("0::/")) else {
        return missing("unified_membership_unavailable");
    };
    let group = Path::new(group);
    if !group.components().all(|part| matches!(part, Component::Normal(_)))
        || !group.file_name().is_some_and(|leaf| leaf.to_string_lossy().contains(id))
    {
        return missing("owned_leaf_unconfirmed");
    }
    let root = Path::new("/sys/fs/cgroup").join(group);
    let Ok(before) = std::fs::metadata(&root) else { return missing("leaf_unavailable") };
    let mut files = std::collections::BTreeMap::new();
    for name in ["memory.events.local", "memory.events"] {
        let Ok(text) = std::fs::read_to_string(root.join(name)) else { continue };
        let counters = text
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once(' ')?;
                if !matches!(key, "low" | "high" | "max" | "oom" | "oom_kill" | "oom_group_kill") {
                    return None;
                }
                let number = value.parse::<u64>().ok()?;
                Some((key.to_owned(), number.to_string()))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        if !counters.is_empty() {
            files.insert(name, counters);
        }
    }
    let mut limits = std::collections::BTreeMap::new();
    for name in ["memory.max", "memory.swap.max", "pids.max"] {
        if let Ok(text) = std::fs::read_to_string(root.join(name)) {
            let value = text.trim();
            if value == "max" || value.parse::<u64>().is_ok() {
                limits.insert(name, value.to_owned());
            }
        }
    }
    let Ok(after) = std::fs::metadata(&root) else { return missing("leaf_disappeared") };
    if before.dev() != after.dev() || before.ino() != after.ino() {
        return missing("leaf_identity_changed");
    }
    let Ok(current) = docker.inspect_container(id, None).await else { return missing("owner_recheck_unavailable") };
    if current.id.as_deref() != Some(id)
        || !current
            .state
            .as_ref()
            .is_some_and(|current| current.pid == Some(pid) && current.started_at == state.started_at)
    {
        return missing("owner_identity_changed");
    }
    if files.is_empty() {
        return missing("counters_unavailable");
    }
    serde_json::json!({"observed": true, "files": files, "limits": limits})
}
