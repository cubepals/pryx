// SPDX-License-Identifier: FSL-1.1-ALv2
//! OOM evidence from Docker's authenticated event API. gVisor can emit `oom` before `die`
//! without retaining OOMKilled in inspect. Evidence belongs to one immutable container ID and
//! one start, never to an exit code, reusable name or another generation. Historical reads
//! recover an inspect-before-event race and daemon restart while Docker retains the event.
use super::*;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Evidence {
    at: OffsetDateTime,
    name: Option<String>,
}

#[derive(Clone, Default)]
pub(super) struct Observations(Arc<Mutex<HashMap<String, Evidence>>>);

impl Observations {
    pub(super) fn event(&self, event: &bollard::models::EventMessage) {
        let Some(actor) = &event.actor else { return };
        let Some(id) = actor.id.as_ref() else { return };
        if event.action.as_deref() == Some("destroy") {
            self.0.lock().unwrap().remove(id);
            return;
        }
        if event.action.as_deref() != Some("oom") {
            return;
        }
        let Some(at) = event.time_nano.and_then(|ns| OffsetDateTime::from_unix_timestamp_nanos(i128::from(ns)).ok())
        else {
            return;
        };
        let name = actor.attributes.as_ref().and_then(|attrs| attrs.get("name")).cloned();
        let mut held = self.0.lock().unwrap();
        let entry = held.entry(id.clone()).or_insert(Evidence { at, name });
        entry.at = entry.at.max(at);
    }

    pub(super) fn forget(&self, name: &str) {
        self.0.lock().unwrap().retain(|id, evidence| id != name && evidence.name.as_deref() != Some(name));
    }

    fn reported(&self, info: &ContainerInfo) -> bool {
        info.started_at.is_some_and(|start| {
            self.0
                .lock()
                .unwrap()
                .get(&info.id)
                .is_some_and(|seen| seen.at >= start && info.finished_at.is_none_or(|finish| seen.at <= finish))
        })
    }

    pub(super) async fn inspect(&self, docker: &Docker, info: &mut ContainerInfo) -> Result<(), RuntimeError> {
        if info.oom_killed || !matches!(info.status, ContainerStatus::Exited | ContainerStatus::Dead) {
            return Ok(());
        }
        if !self.reported(info)
            && let Some(start) = info.started_at
        {
            use time::format_description::well_known::Rfc3339;
            let since = start.format(&Rfc3339).map_err(|error| RuntimeError::Other(error.to_string()))?;
            let until =
                OffsetDateTime::now_utc().format(&Rfc3339).map_err(|error| RuntimeError::Other(error.to_string()))?;
            let filter = HashMap::from([
                ("type".to_owned(), vec!["container".to_owned()]),
                ("container".to_owned(), vec![info.id.clone()]),
                ("event".to_owned(), vec!["oom".to_owned()]),
            ]);
            let options = EventsOptionsBuilder::new().since(&since).until(&until).filters(&filter).build();
            let mut events = docker.events(Some(options));
            quick("inspect Docker OOM events", async {
                while let Some(event) = events.next().await {
                    self.event(&event?);
                }
                Ok(())
            })
            .await?;
        }

        info.oom_killed = self.reported(info);
        if info.oom_killed
            && let Some(evidence) = self.0.lock().unwrap().get_mut(&info.id)
        {
            evidence.name = Some(info.name.clone());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oom_evidence_belongs_to_the_container_and_its_current_start() {
        let cache = Observations::default();
        let start = OffsetDateTime::now_utc();
        let event = serde_json::from_value(serde_json::json!({
            "Action": "oom", "Actor": {"ID": "container-a", "Attributes": {"name": "reused"}},
            "timeNano": (start + time::Duration::seconds(1)).unix_timestamp_nanos() as i64
        }))
        .unwrap();
        cache.event(&event);
        let mut info = container::info_from(Default::default());
        info.id = "container-a".into();
        info.started_at = Some(start);
        assert!(cache.reported(&info));
        info.id = "container-b".into();
        assert!(!cache.reported(&info), "a name never carries evidence to a replacement");
        info.id = "container-a".into();
        info.started_at = Some(start + time::Duration::seconds(2));
        assert!(!cache.reported(&info), "a restart clears the prior run's evidence");
        cache.forget("reused");
        info.started_at = Some(start);
        assert!(!cache.reported(&info), "removal retires the cache entry");
    }

    #[tokio::test]
    async fn a_fresh_backend_recovers_oom_from_the_container_event_history() {
        use axum::{Router, extract::Request, routing::any};
        use bollard::models::{ContainerInspectResponse, ContainerState, EventActor, EventMessage, HostConfig};
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let started = OffsetDateTime::now_utc() - time::Duration::seconds(10);
        let stamp = started + time::Duration::seconds(1);
        let inspected_start = Arc::new(Mutex::new(started));
        let changed = inspected_start.clone();
        let queried = Arc::new(AtomicBool::new(false));
        let checked = queried.clone();
        let app = Router::new().fallback(any(move |request: Request| {
            let start = *changed.lock().unwrap();
            let checked = checked.clone();
            async move {
                if request.uri().path().ends_with("/events") {
                    let query = request.uri().query().unwrap();
                    assert!(query.contains("container-a") && query.contains("since=") && query.contains("until="));
                    checked.store(true, Ordering::SeqCst);
                    let event = EventMessage {
                        action: Some("oom".into()),
                        actor: Some(EventActor { id: Some("container-a".into()), ..Default::default() }),
                        time_nano: Some(stamp.unix_timestamp_nanos() as i64),
                        ..Default::default()
                    };
                    serde_json::to_string(&event).unwrap() + "\n"
                } else {
                    let inspected = ContainerInspectResponse {
                        id: Some("container-a".into()),
                        name: Some("/owned".into()),
                        host_config: Some(HostConfig { runtime: Some(GVISOR_RUNTIME.into()), ..Default::default() }),
                        state: Some(ContainerState {
                            status: Some(bollard::models::ContainerStateStatusEnum::EXITED),
                            oom_killed: Some(false),
                            exit_code: Some(crate::runtime::exit_by(libc::SIGKILL)),
                            started_at: Some(start.format(&time::format_description::well_known::Rfc3339).unwrap()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    };
                    serde_json::to_string(&inspected).unwrap()
                }
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime = DockerRuntime::new(&socket).unwrap().gvisor();
        let recovered = runtime.inspect("owned").await.unwrap().unwrap();
        assert!(recovered.oom_killed && queried.load(Ordering::SeqCst));
        *inspected_start.lock().unwrap() = stamp + time::Duration::seconds(1);
        assert!(
            !runtime.inspect("owned").await.unwrap().unwrap().oom_killed,
            "old event cannot mark a restarted container"
        );
        server.abort();
    }
}
