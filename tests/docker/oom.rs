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
    // Linux include/uapi/linux/oom.h: prefer the allocator as the memory controller's victim.
    const OOM_SCORE_ADJ_MAX: i32 = 1000;
    let mut spec = shell(
        &format!(
            "sleep 1; mkfifo /tmp/oom-input /tmp/oom-observed || exit 1; \
             sh -c 'echo {OOM_SCORE_ADJ_MAX} > /proc/self/oom_score_adj || exit 1; exec head -c 400m /dev/zero' > /tmp/oom-input & producer=$!; \
             sh -c 'echo {OOM_SCORE_ADJ_MAX} > /proc/self/oom_score_adj || exit 1; exec tail /tmp/oom-input' > /dev/null & \
             allocator=$!; wait \"$allocator\"; status=$?; wait \"$producer\"; \
             printf 'ready\\n' > /data/oom-ready || exit 1; \
             read -r observed < /tmp/oom-observed || exit 1; exit \"$status\""
        ),
        64,
    );
    spec.restart.max_retries = 0;
    m.ensure(id("oom"), spec, Precondition::None, None).await.unwrap();
    m.start(id("oom"), None).await.unwrap();
    // Both allocation children precede PID 1 as victims; the producer can fault while the killed
    // allocator's memory is still being reclaimed. Acknowledge only after both children exit,
    // so the test never spawns its exec acknowledgement into that remaining pressure.
    // Block without forking after allocation: spawning sleeps under pressure can also kill PID 1.
    // Keep PID 1 alive until Docker acknowledges the allocating child's OOM. The kernel may
    // also end PID 1 first; either path must retain the OOM flag and the killed allocator's exit.
    let locate = m.view(&id("oom")).unwrap().locate;
    let ready = std::path::Path::new(&locate.data_dir).join("oom-ready");
    let name = locate.container_name;
    let api = bollard::Docker::connect_with_unix(
        DockerConfig::default().socket.to_str().unwrap(),
        2,
        bollard::API_DEFAULT_VERSION,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut release_attempted = false;
    let mut saw_oom = false;
    while Instant::now() < deadline && !saw_oom {
        if !release_attempted
            && api.inspect_container(&name, None).await.unwrap().state.is_some_and(|s| s.oom_killed == Some(true))
            && std::fs::symlink_metadata(&ready).is_ok_and(|info| info.is_file())
        {
            release_attempted = true;
            let release = m
                .exec(
                    id("oom"),
                    ExecRequest {
                        command: vec!["sh".into(), "-c".into(), "printf 'observed\\n' > /tmp/oom-observed".into()],
                        timeout_seconds: 2,
                    },
                    None,
                    None,
                )
                .await;
            match release {
                Ok(release) => {
                    assert_eq!(release.exit_code, Some(0), "the OOM observation is acknowledged once: {release:?}")
                }
                Err(NodeError::Conflict { code: ConflictCode::NotRunning, .. }) => {}
                Err(error) => panic!("OOM acknowledgement failed: {error}"),
            }
        }
        m.stats(&id("oom")).await.ok();
        let view = m.view(&id("oom")).unwrap();
        saw_oom = view.exit.as_ref().is_some_and(|e| e.oom_killed);
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let view = m.view(&id("oom")).unwrap();
    if !saw_oom {
        let mut inspect = tokio::process::Command::new("docker");
        inspect.kill_on_drop(true).args([
            "inspect",
            "--format",
            "{{json .State}} memory={{.HostConfig.Memory}} swap={{.HostConfig.MemorySwap}}",
            &view.locate.container_name,
        ]);
        let diagnostic = tokio::time::timeout(Duration::from_secs(5), inspect.output())
            .await
            .map_err(|e| e.to_string())
            .and_then(|out| out.map_err(|e| e.to_string()))
            .map(|out| format!("{} {}", out.status, String::from_utf8_lossy(&out.stdout)));
        eprintln!("OOM_DOCKER_STATE release_attempted={release_attempted} {diagnostic:?}");
    }
    assert!(
        saw_oom,
        "the kernel's OOM kill inside the limit was observed: state={:?} exit={:?}",
        view.state, view.exit
    );
    assert_eq!(view.exit.unwrap().code, pryxd::runtime::exit_by(libc::SIGKILL));
    cleanup(&m, &["oom"]).await;
}
