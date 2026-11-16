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
