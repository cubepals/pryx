// SPDX-License-Identifier: FSL-1.1-ALv2
//! The real engine memory bound and truthful OOM reporting. Lifecycle cases live in the parent.
use super::*;

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
        let state = inspect.state.unwrap_or_default();
        eprintln!(
            "OOM_FAILURE_STATE {}",
            serde_json::json!({
                "id": inspect.id, "status": state.status, "oomKilled": state.oom_killed,
                "exitCode": state.exit_code, "startedAt": state.started_at, "finishedAt": state.finished_at,
            })
        );
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
        Ok::<_, bollard::errors::Error>(())
    };
    match tokio::time::timeout(Duration::from_secs(3), capture).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => eprintln!("OOM_FAILURE_EVIDENCE engine read unavailable"),
        Err(_) => eprintln!("OOM_FAILURE_EVIDENCE capture deadline"),
    }
}
