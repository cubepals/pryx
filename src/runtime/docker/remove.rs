// SPDX-License-Identifier: FSL-1.1-ALv2
//! Container retirement and owned runtime metadata cleanup. Receipt lock inodes are retired
//! under their index lock so a reconstructed backend cannot acquire a replacement inode.
use super::*;

pub(super) async fn remove(runtime: &DockerRuntime, name: &str) -> Result<(), RuntimeError> {
    runtime.relays.stop(name).await;
    let network = if runtime.info.lock().unwrap().as_ref().is_some_and(|info| info.podman) {
        runtime.inspect(name).await?.and_then(|info| info.labels.get(podman::NETWORK_LABEL).cloned())
    } else {
        None
    };
    // v: anonymous volumes an image declares go with the container instead of leaking; the
    // workload's data is a bind mount, which removal never touches.
    let journal_id = if runtime.journal_context().is_some() {
        quick("inspect removed receipt owner", runtime.docker()?.inspect_container(name, None))
            .await
            .ok()
            .and_then(|owner| owner.id)
    } else {
        None
    };
    let options = RemoveContainerOptionsBuilder::new().force(true).v(true).build();
    match quick("remove", runtime.docker()?.remove_container(name, Some(options))).await {
        Err(RuntimeError::NotFound(_)) | Ok(()) => {
            runtime.oom.forget(name);
            if let (Some(journal), Some(id)) = (runtime.journal_context(), journal_id) {
                journal.retire(&id).await.ok();
            }
            runtime.egress.stop(runtime, name).await?;
            if let Some(network) = network {
                match podman::delete_network(runtime, &network).await {
                    Ok(_) | Err(RuntimeError::NotFound(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}
