// SPDX-License-Identifier: FSL-1.1-ALv2
//! Docker start ownership. The engine receives a start once; lost responses are confirmed
//! through observation rather than a repeated mutation.
use super::*;

impl DockerRuntime {
    pub(super) async fn start_owned(&self, name: &str) -> Result<(), RuntimeError> {
        if let Err(error) = quick("start", self.docker()?.start_container(name, None)).await {
            // A lost response does not undo Docker's mutation. Confirm its current state once;
            // never issue a second start, and preserve the timeout unless it is already running.
            if matches!(error, RuntimeError::Timeout(_))
                && matches!(self.inspect(name).await, Ok(Some(info)) if info.status == ContainerStatus::Running)
            {
                tracing::warn!("confirmed running after Docker start timeout without repeating start");
                return Ok(());
            }
            return Err(error);
        }
        self.inspect(name).await?;
        Ok(())
    }
}
