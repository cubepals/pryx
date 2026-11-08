// SPDX-License-Identifier: FSL-1.1-ALv2
//! Measure the real baseline agent's missing native endpoints, without inventing baseline
//! screenshots for APIs that did not exist. Run against a binary built from the recorded commit.
use pryx_agent::auth::{Claims, Scope, SigningKey, TOKEN_LIFETIME_SECONDS};
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[tokio::main]
async fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let argv: Vec<_> = std::env::args().collect();
    let binary = &argv[1];
    let output = &argv[2];
    let data = tempfile::tempdir().unwrap();
    let (key, secret) = SigningKey::generate().unwrap();
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut server = Server(
        Command::new(binary)
            .args([
                "--sandbox",
                "before",
                "--epoch",
                "1",
                "--data",
                data.path().to_str().unwrap(),
                "--domain",
                "example.test",
                "--port",
                &port.to_string(),
                "--secret-env",
                "PRYX_BEFORE_SECRET",
                "--keep-alive",
            ])
            .env("PRYX_BEFORE_SECRET", secret)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let now = pryx_agent::server::now();
    let token = key
        .sign(&Claims {
            sandbox: "before".into(),
            epoch: 1,
            issued_at: now,
            expires_at: now + TOKEN_LIFETIME_SECONDS,
            scope: Scope::Sandbox,
            max_ports: 2,
            max_exec_seconds: None,
            file: None,
        })
        .unwrap();
    let origin = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    for _ in 0..100 {
        if client.get(format!("{origin}/_pryx/health")).bearer_auth(&token).send().await.is_ok() {
            break;
        }
        assert!(server.0.try_wait().unwrap().is_none(), "Baseline agent exited");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut samples = serde_json::Map::new();
    for path in ["/_pryx/desktop/screenshot", "/_pryx/browser/screenshot", "/_pryx/ssh", "/_pryx/mounts"] {
        let mut elapsed = Vec::new();
        let mut statuses = Vec::new();
        for _ in 0..30 {
            let start = Instant::now();
            let response = client
                .post(format!("{origin}{path}"))
                .bearer_auth(&token)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body("{}")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
            statuses.push(response.status().as_u16());
            response.bytes().await.unwrap();
            elapsed.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.insert(path.into(),serde_json::json!({"n":elapsed.len(),"status":statuses,"milliseconds":elapsed,"outcome":"unsupported before K18"}));
    }
    let document = serde_json::json!({"baselineCommit":"8bfb636","scope":"authenticated baseline agent endpoints, not invented screenshot timings","samples":samples});
    std::fs::write(output, serde_json::to_string_pretty(&document).unwrap() + "\n").unwrap();
}
