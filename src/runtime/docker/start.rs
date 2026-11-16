// SPDX-License-Identifier: FSL-1.1-ALv2
//! Docker start ownership. The engine receives a start once; lost responses are confirmed
//! through observation rather than a repeated mutation. Exec lifetime remains in exec.rs.
use super::*;

impl DockerRuntime {
    pub(super) async fn start_owned(&self, name: &str) -> Result<(), RuntimeError> {
        let id = quick("inspect start identity", self.docker()?.inspect_container(name, None))
            .await?
            .id
            .ok_or_else(|| RuntimeError::Other("start target has no immutable identity".into()))?;
        let generation = self.starts.lock(&id).await;
        let name = id.as_str();
        if let Err(error) = quick("start", self.docker()?.start_container(name, None)).await {
            // A lost response does not undo Docker's mutation. Confirm its current state once;
            // never issue a second start, and preserve the timeout unless it is already running.
            if matches!(error, RuntimeError::Timeout(_))
                && matches!(
                    quick("confirm start", self.docker()?.inspect_container(name, None)).await,
                    Ok(info) if gvisor::owns_container(self.oci_runtime, &info)
                        && info.state.as_ref().and_then(|state| state.running) == Some(true)
                )
            {
                tracing::warn!("confirmed running after Docker start timeout without repeating start");
            } else {
                return Err(error);
            }
        }
        // Confirmation above is read-only while mutation authority is held. Relay adoption
        // can retire an orphan from this start, so it must acquire that authority separately.
        drop(generation);
        self.inspect(name).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::Request, routing::any};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn idempotent_start_can_retire_an_abandoned_ingress_owner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let kills = Arc::new(AtomicUsize::new(0));
        let starts = Arc::new(AtomicUsize::new(0));
        let probes = Arc::new(AtomicUsize::new(0));
        let observed = (kills.clone(), starts.clone(), probes.clone());
        let timestamp = OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap();
        let bindings = serde_json::to_string(&vec![crate::runtime::PortBinding {
            container_port: 8080,
            host_port: 0,
            host_ips: vec![std::net::Ipv4Addr::LOCALHOST.into()],
            protocol: crate::protocol::Proto::Tcp,
        }])
        .unwrap();
        let record = crate::labels::private_record(
            "alpine:3.22",
            &serde_json::from_str::<Vec<crate::runtime::PortBinding>>(&bindings).unwrap(),
        );
        let app = Router::new().fallback(any(move |request: Request| {
            let (kills, starts, probes) = observed.clone();
            let timestamp = timestamp.clone();
            let bindings = bindings.clone();
            let record = record.clone();
            async move {
                let route = request.uri().path();
                if route.ends_with("/start") {
                    starts.fetch_add(1, Ordering::SeqCst);
                    // A real Engine returns this for an already-running immutable ID.
                    return (StatusCode::NOT_MODIFIED, Json(serde_json::json!({})));
                }
                if route.ends_with("/kill") {
                    kills.fetch_add(1, Ordering::SeqCst);
                    return (StatusCode::NO_CONTENT, Json(serde_json::json!({})));
                }
                if route.contains("/exec/") {
                    if probes.fetch_add(1, Ordering::SeqCst) > 0 {
                        // Losing the read-only completion observation cannot establish EOF.
                        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"message": "engine unavailable"})));
                    }
                    return (StatusCode::OK, Json(serde_json::json!({"ContainerID": "owned", "Running": true,
                        "ProcessConfig": {"entrypoint": pryx_agent::contract::BINARY_PATH,
                            "arguments": [pryx_agent::exec_ingress::MODE, "tcp", "8080"]}})));
                }
                let running = kills.load(Ordering::SeqCst) == 0;
                (StatusCode::OK, Json(serde_json::json!({"Id": "owned", "Name": "/owned",
                    "State": {"Running": running, "Status": if running {"running"} else {"exited"}, "StartedAt": timestamp},
                    "Config": {"Labels": {crate::labels::LABEL_INTERNAL_BINDINGS: bindings, crate::labels::LABEL_RECORD: record}}, "ExecIDs": ["abandoned"]})))
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime = DockerRuntime::new(&path).unwrap();
        *runtime.info.lock().unwrap() = Some(RuntimeInfo { rootless: true, ..Default::default() });
        let error = tokio::time::timeout(Duration::from_secs(2), runtime.start("owned"))
            .await
            .expect("adoption must not wait on its own start lock")
            .unwrap_err();
        assert!(error.to_string().contains("old container start was retired"), "{error}");
        assert_eq!(starts.load(Ordering::SeqCst), 1, "never repeat a start mutation");
        assert_eq!(kills.load(Ordering::SeqCst), 1, "retire the abandoned generation exactly once");
        server.abort();
    }
}
