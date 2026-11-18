// SPDX-License-Identifier: FSL-1.1-ALv2
//! One engine start mutation. Configured Docker backends durably bind the native systemd
//! invocation; direct callers without that context and Podman retain their observation path.
use super::*;

pub(super) async fn start(runtime: &DockerRuntime, name: &str) -> Result<(), RuntimeError> {
    if runtime.oci_runtime.is_some() || runtime.journal_context().is_some() {
        return journal_start(runtime, name).await;
    }
    if let Err(error) = quick("start", runtime.docker()?.start_container(name, None)).await {
        if matches!(error, RuntimeError::Timeout(_))
            && matches!(runtime.inspect(name).await, Ok(Some(info)) if info.status == ContainerStatus::Running)
        {
            tracing::warn!("confirmed running after Docker start timeout without repeating start");
            return Ok(());
        }
        return Err(error);
    }
    runtime.inspect(name).await?;
    Ok(())
}

async fn journal_start(runtime: &DockerRuntime, name: &str) -> Result<(), RuntimeError> {
    let before = quick("inspect Docker start owner", runtime.docker()?.inspect_container(name, None)).await?;
    if !gvisor::owns_container(runtime.oci_runtime, &before) {
        return Err(RuntimeError::NotFound(name.into()));
    }
    let before = info_from(before);
    let mut capture = None;
    if let Some(context) = runtime.journal_context()
        && context.can_capture()?
    {
        match context.begin(&before).await {
            Ok(held) => capture = Some(held),
            Err(error) => return Err(error),
        }
    }
    // Names can be replaced between inspection and mutation. Start only the immutable ID.
    let result = quick("start", runtime.docker()?.start_container(&before.id, None)).await;
    let actual = quick("confirm Docker start owner", runtime.docker()?.inspect_container(&before.id, None)).await?;
    let actual = info_from(actual);
    if actual.id != before.id {
        return Err(oom_journal::failure("Docker start owner changed"));
    }
    if let (Some(context), Some(held)) = (runtime.journal_context(), &mut capture)
        && context.finish(runtime, held, &before, &actual, &result).await.is_err()
    {
        tracing::debug!("native journal scope invocation is unknown");
    }
    drop(capture);
    match result {
        Ok(()) => (),
        Err(RuntimeError::Timeout(_)) if actual.status == ContainerStatus::Running => {
            tracing::warn!("confirmed running after Docker start timeout without repeating start");
        }
        Err(error) => return Err(error),
    }
    runtime.inspect(&before.id).await?;
    Ok(())
}
