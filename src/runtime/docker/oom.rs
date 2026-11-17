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

    fn during_run(info: &ContainerInfo, at: OffsetDateTime) -> bool {
        info.started_at.is_some_and(|start| at >= start && info.finished_at.is_none_or(|finish| at <= finish))
    }

    fn reported(&self, info: &ContainerInfo) -> bool {
        self.0.lock().unwrap().get(&info.id).is_some_and(|seen| Self::during_run(info, seen.at))
    }

    fn event_reports(info: &ContainerInfo, event: &bollard::models::EventMessage) -> bool {
        event.action.as_deref() == Some("oom")
            && event.actor.as_ref().and_then(|actor| actor.id.as_deref()) == Some(info.id.as_str())
            && event
                .time_nano
                .and_then(|ns| OffsetDateTime::from_unix_timestamp_nanos(i128::from(ns)).ok())
                .is_some_and(|at| Self::during_run(info, at))
    }

    pub(super) async fn inspect(&self, docker: &Docker, info: &mut ContainerInfo) -> Result<(), RuntimeError> {
        if info.oom_killed || !matches!(info.status, ContainerStatus::Exited | ContainerStatus::Dead) {
            return Ok(());
        }
        // Preserve this run's proof even if a concurrent restart replaces the shared cache.
        let mut reported = self.reported(info);
        if !reported && let Some(start) = info.started_at {
            // Moby's time.ParseTimestamps accepts numeric seconds.nanoseconds. Keep this
            // cutoff in the past when it reaches the engine: a future bound makes every
            // ordinary stopped container wait. Exact run scoping stays above.
            let since = start.unix_timestamp().to_string();
            let now = OffsetDateTime::now_utc();
            let until = format!("{}.{:09}", now.unix_timestamp(), now.nanosecond());
            let filter = HashMap::from([
                ("type".to_owned(), vec!["container".to_owned()]),
                ("container".to_owned(), vec![info.id.clone()]),
                ("event".to_owned(), vec!["oom".to_owned()]),
            ]);
            let options = EventsOptionsBuilder::new().since(&since).until(&until).filters(&filter).build();
            let mut events = docker.events(Some(options));
            quick("inspect Docker OOM events", async {
                while let Some(event) = events.next().await {
                    let event = event?;
                    reported |= Self::event_reports(info, &event);
                    self.event(&event);
                }
                Ok(())
            })
            .await?;
        }

        if !reported && let Some(start) = info.started_at {
            // Moby publishes an OOM event after updating the owning container state. Its
            // publication timestamp can follow containerd's FinishedAt; do not stretch the
            // event window into another start. Confirm the owner's flag once for this start.
            let current = quick("confirm Docker OOM state", docker.inspect_container(&info.id, None)).await?;
            let current = container::info_from(current);
            if current.id != info.id || current.started_at != Some(start) {
                return Err(RuntimeError::Other("Docker container start changed while observing OOM state".into()));
            }
            reported = current.oom_killed;
        }

        info.oom_killed = reported || self.reported(info);
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
        info.finished_at = Some(start);
        assert!(!cache.reported(&info), "evidence after the run ended is not this run's OOM");
        info.finished_at = None;
        info.id = "container-b".into();
        assert!(!cache.reported(&info), "a name never carries evidence to a replacement");
        info.id = "container-a".into();
        info.started_at = Some(start + time::Duration::seconds(2));
        assert!(!cache.reported(&info), "a restart clears the prior run's evidence");
        cache.forget("reused");
        info.started_at = Some(start);
        assert!(!cache.reported(&info), "removal retires the cache entry");
    }

    #[test]
    fn only_a_timestamped_oom_event_is_evidence_and_destroy_retires_it() {
        let cache = Observations::default();
        let start = OffsetDateTime::now_utc();
        let mut event: bollard::models::EventMessage = serde_json::from_value(serde_json::json!({
            "Action": "die", "Actor": {"ID": "container-a"},
            "timeNano": start.unix_timestamp_nanos() as i64
        }))
        .unwrap();
        let mut info = container::info_from(Default::default());
        info.id = "container-a".into();
        info.started_at = Some(start);
        cache.event(&event);
        assert!(!cache.reported(&info), "a die event never claims OOM");
        event.action = Some("oom".into());
        event.time_nano = None;
        cache.event(&event);
        assert!(!cache.reported(&info), "an undated event cannot belong to a start");
        event.time_nano = Some(start.unix_timestamp_nanos() as i64);
        cache.event(&event);
        assert!(cache.reported(&info));
        event.action = Some("destroy".into());
        cache.event(&event);
        assert!(!cache.reported(&info));
    }

    #[tokio::test]
    async fn sigkill_without_engine_oom_evidence_is_not_reported_as_oom() {
        use axum::{Router, extract::Request, routing::any};
        use bollard::models::{ContainerInspectResponse, ContainerState, HostConfig};
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let start = OffsetDateTime::now_utc() - time::Duration::seconds(10);
        let app = Router::new().fallback(any(move |request: Request| async move {
            if request.uri().path().ends_with("/events") {
                String::new()
            } else {
                serde_json::to_string(&ContainerInspectResponse {
                    id: Some("container-a".into()),
                    host_config: Some(HostConfig { runtime: Some(GVISOR_RUNTIME.into()), ..Default::default() }),
                    state: Some(ContainerState {
                        status: Some(bollard::models::ContainerStateStatusEnum::EXITED),
                        oom_killed: Some(false),
                        exit_code: Some(crate::runtime::exit_by(libc::SIGKILL)),
                        started_at: Some(start.format(&time::format_description::well_known::Rfc3339).unwrap()),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .unwrap()
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let info = DockerRuntime::new(&socket).unwrap().gvisor().inspect("container-a").await.unwrap().unwrap();
        assert_eq!(info.exit_code, crate::runtime::exit_by(libc::SIGKILL));
        assert!(!info.oom_killed, "SIGKILL is not proof that the memory controller killed the workload");
        server.abort();
    }

    #[tokio::test]
    async fn a_fresh_backend_recovers_oom_from_the_container_event_history() {
        use axum::{Router, extract::Request, response::IntoResponse, routing::any};
        use bollard::models::{ContainerInspectResponse, ContainerState, EventActor, EventMessage, HostConfig};
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let stamp = OffsetDateTime::now_utc();
        let started = stamp - time::Duration::nanoseconds(1);
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
                    let url = reqwest::Url::parse(&format!("http://localhost{}", request.uri())).unwrap();
                    let bounds = url
                        .query_pairs()
                        .filter(|(key, _)| key == "since" || key == "until")
                        .map(|(key, value)| {
                            // Moby's numeric HTTP format retains nanoseconds; RFC3339 is
                            // accepted by the CLI parser, not this endpoint.
                            let (seconds, nanos) = value.split_once('.').unwrap_or((&value, "0"));
                            let seconds = seconds.parse::<i64>()?;
                            let nanos = format!("{nanos:0<9}").parse::<i64>()?;
                            let at = OffsetDateTime::from_unix_timestamp(seconds).unwrap()
                                + time::Duration::nanoseconds(nanos);
                            Ok::<_, std::num::ParseIntError>((key.into_owned(), at))
                        })
                        .collect::<Result<HashMap<_, _>, _>>();
                    let bounds = match bounds {
                        Ok(bounds) => bounds,
                        Err(error) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                axum::Json(serde_json::json!({
                                    "message": error.to_string()
                                })),
                            )
                                .into_response();
                        }
                    };
                    checked.store(true, Ordering::SeqCst);
                    let event = EventMessage {
                        action: Some("oom".into()),
                        actor: Some(EventActor { id: Some("container-a".into()), ..Default::default() }),
                        time_nano: Some(stamp.unix_timestamp_nanos() as i64),
                        ..Default::default()
                    };
                    let included = stamp >= bounds["since"] && stamp <= bounds["until"];
                    let body = if included { serde_json::to_string(&event).unwrap() + "\n" } else { String::new() };
                    body.into_response()
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
                    serde_json::to_string(&inspected).unwrap().into_response()
                }
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime = DockerRuntime::new(&socket).unwrap().gvisor();
        let recovered = runtime.inspect("owned").await.unwrap().unwrap();
        assert!(recovered.oom_killed && queried.load(Ordering::SeqCst));
        *inspected_start.lock().unwrap() = stamp + time::Duration::nanoseconds(1);
        assert!(
            !runtime.inspect("owned").await.unwrap().unwrap().oom_killed,
            "old event cannot mark a restarted container"
        );
        server.abort();
    }
    #[tokio::test]
    async fn a_history_query_keeps_its_run_evidence_when_a_later_run_ooms() {
        use axum::{Router, extract::Request, routing::any};
        use bollard::models::{ContainerInspectResponse, ContainerState, EventActor, EventMessage, HostConfig};
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let start_a = OffsetDateTime::now_utc() - time::Duration::seconds(1);
        let oom_a = start_a + time::Duration::nanoseconds(1);
        let finish_a = oom_a + time::Duration::nanoseconds(1);
        let start_b = finish_a + time::Duration::nanoseconds(1);
        let oom_b = start_b + time::Duration::nanoseconds(1);
        let run = Arc::new(Mutex::new((start_a, finish_a)));
        let inspected_run = run.clone();
        let app = Router::new().fallback(any(move |request: Request| {
            let (start, finish) = *inspected_run.lock().unwrap();
            async move {
                if request.uri().path().ends_with("/events") {
                    // The finite upper bound can keep A's stream open through B's OOM.
                    [oom_a, oom_b]
                        .into_iter()
                        .map(|stamp| {
                            serde_json::to_string(&EventMessage {
                                action: Some("oom".into()),
                                actor: Some(EventActor { id: Some("container-a".into()), ..Default::default() }),
                                time_nano: Some(stamp.unix_timestamp_nanos() as i64),
                                ..Default::default()
                            })
                            .unwrap()
                                + "\n"
                        })
                        .collect::<String>()
                } else {
                    let formatted =
                        |stamp: OffsetDateTime| stamp.format(&time::format_description::well_known::Rfc3339).unwrap();
                    serde_json::to_string(&ContainerInspectResponse {
                        id: Some("container-a".into()),
                        host_config: Some(HostConfig { runtime: Some(GVISOR_RUNTIME.into()), ..Default::default() }),
                        state: Some(ContainerState {
                            status: Some(bollard::models::ContainerStateStatusEnum::EXITED),
                            oom_killed: Some(false),
                            started_at: Some(formatted(start)),
                            finished_at: Some(formatted(finish)),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .unwrap()
                }
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime = DockerRuntime::new(&socket).unwrap().gvisor();
        let first = runtime.inspect("container-a").await.unwrap().unwrap();
        assert!(first.oom_killed, "run B replacing the cache must not erase run A's matching history");
        *run.lock().unwrap() = (start_b, oom_b);
        assert!(runtime.inspect("container-a").await.unwrap().unwrap().oom_killed, "run B retains its own OOM");
        *run.lock().unwrap() = (oom_b + time::Duration::nanoseconds(1), oom_b + time::Duration::nanoseconds(2));
        assert!(
            !runtime.inspect("container-a").await.unwrap().unwrap().oom_killed,
            "neither prior run's event belongs to the next start"
        );
        server.abort();
    }

    #[tokio::test]
    async fn a_late_engine_oom_flag_is_confirmed_for_the_same_start() {
        let (result, confirmed) = late_owner_state(false).await;
        let info = result.unwrap().unwrap();
        assert!(info.oom_killed, "the owner retains OOM after publishing outside the process lifetime");
        assert!(confirmed, "the confirmation must use the immutable ID rather than its reusable name");
    }

    #[tokio::test]
    async fn a_late_engine_oom_flag_cannot_cross_a_container_restart() {
        let (result, confirmed) = late_owner_state(true).await;
        assert!(matches!(result, Err(RuntimeError::Other(_))), "a replacement start cannot supply this exit's OOM");
        assert!(confirmed);
    }

    async fn late_owner_state(restarted: bool) -> (Result<Option<ContainerInfo>, RuntimeError>, bool) {
        use axum::{Router, extract::Request, routing::any};
        use bollard::models::{ContainerInspectResponse, ContainerState, EventActor, EventMessage};
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("engine.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let start = OffsetDateTime::now_utc() - time::Duration::seconds(3);
        let finish = start + time::Duration::seconds(1);
        let published = finish + time::Duration::nanoseconds(1);
        let history = Arc::new(AtomicBool::new(false));
        let queried = history.clone();
        let confirmed = Arc::new(AtomicBool::new(false));
        let checked = confirmed.clone();
        let app = Router::new().fallback(any(move |request: Request| {
            let queried = queried.clone();
            let checked = checked.clone();
            async move {
                if request.uri().path().ends_with("/events") {
                    // Moby sets the owning state before its publication-time event. The event
                    // itself remains outside the strict completed-run timestamp boundary.
                    queried.store(true, Ordering::SeqCst);
                    serde_json::to_string(&EventMessage {
                        action: Some("oom".into()),
                        actor: Some(EventActor { id: Some("container-a".into()), ..Default::default() }),
                        time_nano: Some(published.unix_timestamp_nanos() as i64),
                        ..Default::default()
                    })
                    .unwrap()
                        + "\n"
                } else {
                    let after = queried.load(Ordering::SeqCst);
                    if after {
                        checked.store(request.uri().path().ends_with("/containers/container-a/json"), Ordering::SeqCst);
                    }
                    let start = if after && restarted { published + time::Duration::nanoseconds(1) } else { start };
                    let formatted =
                        |stamp: OffsetDateTime| stamp.format(&time::format_description::well_known::Rfc3339).unwrap();
                    serde_json::to_string(&ContainerInspectResponse {
                        id: Some("container-a".into()),
                        name: Some("/reused".into()),
                        state: Some(ContainerState {
                            status: Some(bollard::models::ContainerStateStatusEnum::EXITED),
                            oom_killed: Some(after),
                            started_at: Some(formatted(start)),
                            finished_at: Some(formatted(if after && restarted { start } else { finish })),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .unwrap()
                }
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime = DockerRuntime::new(&socket).unwrap();
        *runtime.info.lock().unwrap() = Some(RuntimeInfo::default());
        let result = runtime.inspect("reused").await;
        server.abort();
        (result, confirmed.load(Ordering::SeqCst))
    }
}
