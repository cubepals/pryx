// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Resource-policy acceptance on the actual selected engine. Rootful execution must retain hard
//! limits; an engine without controllers must refuse requested limits before creating data.
use super::*;

#[tokio::test]
#[ignore = "needs a real Docker-compatible engine"]
async fn explicit_memory_acceptance_never_removes_available_enforcement() {
    let socket = pryxd::runtime::engine::socket(&DockerConfig::default()).await.unwrap();
    let runtime = Arc::new(DockerRuntime::new(&socket).unwrap());
    let limits = runtime.resource_limits().await.unwrap().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dep = deployment();
    let _cleanup = EngineCleanup { socket: socket.clone(), deployment: dep.clone() };
    let owner = if limits.rootless {
        format!(
            "[workloads]\ndata_owner = \"{}:{}\"\n",
            rustix::process::geteuid().as_raw(),
            rustix::process::getegid().as_raw()
        )
    } else {
        HOST_OWNERSHIP.into()
    };
    let configuration = extra(&dep).replace(HOST_OWNERSHIP, &owner);
    let manager = manager_on(dir.path(), runtime, &configuration);
    assert!(manager.reconcile(Records::FromDisk).await.error.is_none());
    let mut value = support::spec_json();
    value["entrypoint"] = serde_json::json!(["/bin/sh", "-c", "trap 'exit 0' TERM; while :; do sleep 1; done"]);
    value["stop"]["timeoutSeconds"] = 5.into();
    value["resources"] = serde_json::json!({"memoryMb": 256});
    if !limits.memory {
        let refused = manager
            .ensure(id("limited"), serde_json::from_value(value.clone()).unwrap(), Precondition::None, None)
            .await
            .unwrap_err();
        refusal(refused, pryxd::protocol::Guarantee::MemoryLimit);
        assert!(!manager.store.data_dir(&id("limited")).exists());
    }
    value["resources"]["allowUnenforcedMemory"] = true.into();
    if !limits.cpu {
        value["resources"]["cpuMillis"] = pryxd::protocol::MIN_CPU_MILLIS.into();
        let refused = manager
            .ensure(id("cpu-limited"), serde_json::from_value(value.clone()).unwrap(), Precondition::None, None)
            .await
            .unwrap_err();
        refusal(refused, pryxd::protocol::Guarantee::CpuLimit);
        assert!(!manager.store.data_dir(&id("cpu-limited")).exists());
        value["resources"].as_object_mut().unwrap().remove("cpuMillis");
    }
    let made =
        manager.ensure(id("accepted"), serde_json::from_value(value).unwrap(), Precondition::None, None).await.unwrap();
    let view = serde_json::to_value(&made.workload).unwrap();
    assert_eq!(view["guarantees"]["memory-limit"], limits.memory);
    assert_eq!(view["guarantees"]["cpu-limit"], limits.cpu);
    let backend =
        bollard::Docker::connect_with_unix(socket.to_str().unwrap(), 60, bollard::API_DEFAULT_VERSION).unwrap();
    let options = bollard::query_parameters::ListContainersOptionsBuilder::new()
        .all(true)
        .filters(&std::collections::HashMap::from([("label".to_owned(), vec![format!("{LABEL_DEPLOYMENT}={dep}")])]))
        .build();
    let containers = backend.list_containers(Some(options)).await.unwrap();
    assert_eq!(containers.len(), 1, "refusals create no compute");
    let inspected = backend.inspect_container(containers[0].id.as_ref().unwrap(), None).await.unwrap();
    let host = inspected.host_config.unwrap();
    assert_eq!(host.memory.unwrap_or_default(), if limits.memory { 256 * 1024 * 1024 } else { 0 });
    assert_eq!(host.readonly_rootfs, Some(true));
    assert_eq!(host.privileged, Some(false));
    assert!(host.cap_drop.unwrap().iter().any(|cap| cap == "ALL"));
    assert!(
        host.security_opt
            .unwrap()
            .iter()
            .any(|option| option == "no-new-privileges:true" || option == "no-new-privileges")
    );
    manager.start(id("accepted"), None).await.unwrap();
    let result = manager
        .exec(
            id("accepted"),
            ExecRequest {
                command: vec![
                    "sh".into(),
                    "-c".into(),
                    "printf retained > /data/engine-policy; cat /data/engine-policy".into(),
                ],
                timeout_seconds: 10,
            },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.stdout, "retained");
    cleanup(&manager, &["accepted"]).await;
}

fn refusal(error: NodeError, guarantee: pryxd::protocol::Guarantee) {
    let NodeError::Invalid(errors) = error else {
        panic!("unexpected refusal: {error:?}");
    };
    let expected = serde_json::to_value(guarantee).unwrap();
    assert!(serde_json::to_string(&errors).unwrap().contains(expected.as_str().unwrap()));
}

pub(super) struct EngineCleanup {
    pub(super) socket: std::path::PathBuf,
    pub(super) deployment: String,
}
impl Drop for EngineCleanup {
    fn drop(&mut self) {
        let endpoint = format!("unix://{}", self.socket.display());
        let filter = format!("label={LABEL_DEPLOYMENT}={}", self.deployment);
        if let Ok(output) =
            std::process::Command::new("docker").args(["--host", &endpoint, "ps", "-aq", "--filter", &filter]).output()
        {
            let ids = String::from_utf8_lossy(&output.stdout).split_whitespace().map(str::to_owned).collect::<Vec<_>>();
            if output.status.success() && !ids.is_empty() {
                let _ = std::process::Command::new("docker").args(["--host", &endpoint, "rm", "-f"]).args(ids).output();
            }
        }
    }
}
