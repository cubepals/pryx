// SPDX-License-Identifier: FSL-1.1-ALv2
//! Read-only fault injection for the real OOM acceptance. The owned container remains real;
//! only its Docker OOM flag and OOM event history disappear, forcing the native journal path.
use axum::{Router, extract::Request, response::IntoResponse, routing::any};
use bollard::Docker;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) struct EvidenceGap {
    socket: PathBuf,
    server: tokio::task::JoinHandle<()>,
    inspected: Arc<AtomicUsize>,
    history: Arc<AtomicUsize>,
    _directory: tempfile::TempDir,
}

impl EvidenceGap {
    pub(super) async fn new(socket: &Path, owned: &str) -> Self {
        assert_eq!(owned.len(), 64);
        assert!(owned.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let engine = Docker::connect_with_unix(socket.to_str().unwrap(), 3, bollard::API_DEFAULT_VERSION)
            .unwrap()
            .negotiate_version()
            .await
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("read-only-engine.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let inspected = Arc::new(AtomicUsize::new(0));
        let history = Arc::new(AtomicUsize::new(0));
        let (reads, histories, owned) = (inspected.clone(), history.clone(), owned.to_owned());
        let app = Router::new().fallback(any(move |request: Request| {
            let (engine, reads, histories, owned) = (engine.clone(), reads.clone(), histories.clone(), owned.clone());
            async move {
                use axum::http::StatusCode;
                if request.method() != axum::http::Method::GET {
                    return StatusCode::METHOD_NOT_ALLOWED.into_response();
                }
                let path = request.uri().path();
                if path.ends_with(&format!("/containers/{owned}/json")) {
                    return match engine.inspect_container(&owned, None).await {
                        Ok(mut actual) => {
                            assert_eq!(actual.id.as_deref(), Some(owned.as_str()));
                            actual.state.as_mut().expect("the actual container state").oom_killed = Some(false);
                            reads.fetch_add(1, Ordering::SeqCst);
                            axum::Json(actual).into_response()
                        }
                        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
                    };
                }
                if path.ends_with("/events") {
                    let url = reqwest::Url::parse(&format!("http://localhost{}", request.uri())).unwrap();
                    let filters = url.query_pairs().find(|(key, _)| key == "filters").unwrap().1;
                    let filters: serde_json::Value = serde_json::from_str(&filters).unwrap();
                    assert_eq!(filters["type"], serde_json::json!(["container"]));
                    assert_eq!(filters["container"], serde_json::json!([owned]));
                    assert_eq!(filters["event"], serde_json::json!(["oom"]));
                    histories.fetch_add(1, Ordering::SeqCst);
                    return String::new().into_response();
                }
                if path.ends_with("/info") {
                    return match engine.info().await {
                        Ok(actual) => axum::Json(actual).into_response(),
                        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
                    };
                }
                if path.ends_with("/version") {
                    return match engine.version().await {
                        Ok(actual) => axum::Json(actual).into_response(),
                        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
                    };
                }
                StatusCode::NOT_FOUND.into_response()
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { socket, server, inspected, history, _directory: directory }
    }

    pub(super) fn socket(&self) -> &Path {
        &self.socket
    }

    pub(super) fn assert_exercised(&self) {
        assert!(self.inspected.load(Ordering::SeqCst) >= 2, "both Docker owner reads must lose their OOM flag");
        assert!(self.history.load(Ordering::SeqCst) > 0, "the fresh backend must receive empty OOM history");
    }
}

impl Drop for EvidenceGap {
    fn drop(&mut self) {
        self.server.abort();
    }
}
