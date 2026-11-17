// SPDX-License-Identifier: FSL-1.1-ALv2
//! Docker container removal and its existing owned cleanup.
//! This module preserves the existing engine behavior.
use super::*;

impl DockerRuntime {
    pub(super) async fn remove_owned(&self, name: &str) -> Result<(), RuntimeError> {
        self.relays.stop(name).await;
        let network = if self.info.lock().unwrap().as_ref().is_some_and(|info| info.podman) {
            self.inspect(name).await?.and_then(|info| info.labels.get(podman::NETWORK_LABEL).cloned())
        } else {
            None
        };
        // v: anonymous volumes an image declares go with the container instead of leaking; the
        // workload's data is a bind mount, which removal never touches.
        let options = RemoveContainerOptionsBuilder::new().force(true).v(true).build();
        match quick("remove", self.docker()?.remove_container(name, Some(options))).await {
            Err(RuntimeError::NotFound(_)) | Ok(()) => {
                self.oom.forget(name);
                self.egress.stop(self, name).await?;
                if let Some(network) = network {
                    match podman::delete_network(self, &network).await {
                        Ok(_) | Err(RuntimeError::NotFound(_)) => {}
                        Err(error) => return Err(error),
                    }
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}
