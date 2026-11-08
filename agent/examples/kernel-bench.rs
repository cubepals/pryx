// SPDX-License-Identifier: FSL-1.1-ALv2
//! Direct authenticated loopback measurements compare a fresh Python process with a live
//! kernel on the same host. Deployment/template timing is measured by the control suite.
use axum::http::header;
use axum::serve::ListenerExt;
use pryx_agent::{
    auth::{Claims, Scope, SigningKey, TOKEN_LIFETIME_SECONDS},
    paths::DataDir,
    ports::Ports,
    process::{Command, Processes, Start},
    server::{Agent, now, router},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};

async fn request(client: &reqwest::Client, origin: &str, token: &str, path: &str, body: Value) -> String {
    let response = client
        .post(format!("{origin}{path}"))
        .bearer_auth(token)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {text}");
    text
}
fn checked(text: &str, mime: &str) {
    let events: Vec<Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert!(events.iter().any(|event| event["data"].get(mime).is_some()), "{text}");
    assert_eq!(events.last().unwrap()["status"], "ok");
}
#[tokio::main]
async fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let (key, _) = SigningKey::generate().unwrap();
    let token = key
        .sign(&Claims {
            sandbox: "bench".into(),
            epoch: 1,
            issued_at: now(),
            expires_at: now() + TOKEN_LIFETIME_SECONDS,
            scope: Scope::Sandbox,
            max_ports: 2,
            max_exec_seconds: Some(30),
            file: None,
        })
        .unwrap();
    let telemetry = Arc::default();
    let agent = Arc::new(Agent {
        file_tasks: Default::default(),
        shells: Default::default(),
        logs: Arc::new(pryx_agent::logbook::Logbook::open(dir.path()).unwrap()),
        telemetry,
        sandbox: "bench".into(),
        epoch: 1,
        public_key: key.public_key(),
        key,
        data: DataDir::open(dir.path()).unwrap(),
        processes: Processes::new(dir.path().into(), "PRYX_BENCH_SECRET".into()),
        ports: Ports::bind("bench".into(), "example.test".into(), 0).await.unwrap(),
    });
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(
        axum::serve(
            listener.tap_io(|socket| {
                let _ = socket.set_nodelay(true);
            }),
            router(agent.clone()),
        )
        .into_future(),
    );
    let client = reqwest::Client::new();
    let mut samples = serde_json::Map::new();
    let count = std::env::var("PRYX_BENCH_N").map_or(30, |n| n.parse::<usize>().expect("positive sample count"));
    assert!(count > 0);
    let imports = "import numpy as np; import pandas as pd; import matplotlib; matplotlib.use('Agg'); import matplotlib.pyplot as plt; ";
    for (name, code) in [
        ("before_fresh_process_trivial", format!("{imports}print(1+1)")),
        (
            "before_fresh_process_chart",
            format!(
                "{imports}import io,base64; plt.plot([1,2,3]); b=io.BytesIO(); plt.savefig(b,format='png'); print(base64.b64encode(b.getvalue()).decode())"
            ),
        ),
    ] {
        let mut values = Vec::new();
        for _ in 0..count {
            let start = Instant::now();
            let process = agent
                .processes
                .start(
                    Start {
                        command: Command::Argv(vec!["python3".into(), "-c".into(), code.clone()]),
                        env: Default::default(),
                        cwd: None,
                        stdin: false,
                        pty: false,
                        cols: None,
                        rows: None,
                        timeout: Some(30),
                    },
                    &uuid::Uuid::new_v4().to_string(),
                    None,
                )
                .unwrap();
            let result = process.wait().await;
            assert_eq!(result.code, Some(0));
            values.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.insert(name.into(), json!(values));
    }
    let opened = request(&client, &origin, &token, "/_pryx/kernels", json!({"language":"python"})).await;
    let view: Value = serde_json::from_str(&opened).unwrap();
    let id = view["id"].as_str().unwrap();
    for (name, code, mime) in [
        ("after_first_run_after_reuse", "1+1", "application/json"),
        ("after_trivial", "1+1", "application/json"),
        ("after_chart", "plt.plot([1,2,3]); plt.show()", "image/png"),
    ] {
        let mut values = Vec::new();
        for _ in 0..count {
            if name == "after_first_run_after_reuse" {
                request(&client, &origin, &token, "/_pryx/kernels", json!({"language":"python"})).await;
            }
            let start = Instant::now();
            let result =
                request(&client, &origin, &token, &format!("/_pryx/kernels/{id}/execute"), json!({"code":code})).await;
            checked(&result, mime);
            values.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.insert(name.into(), json!(values));
    }
    let mut restarts = Vec::new();
    for _ in 0..count {
        let start = Instant::now();
        request(&client, &origin, &token, &format!("/_pryx/kernels/{id}/restart"), json!({})).await;
        restarts.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.insert("after_restart".into(), json!(restarts));
    let mut summary = serde_json::Map::new();
    for (name, values) in &samples {
        let mut sorted: Vec<f64> = values.as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        sorted.sort_by(f64::total_cmp);
        summary.insert(name.clone(), json!({"n":sorted.len(),"p50Ms":sorted[(sorted.len() * 50).div_ceil(100) - 1],"p90Ms":sorted[(sorted.len() * 90).div_ceil(100) - 1]}));
    }
    let output = json!({"transport":"authenticated agent HTTP loopback","baseline":"fresh Python process with the same imports; pre-K17 kernels unavailable","samples":samples,"summary":summary});
    let path = std::env::args().nth(1).expect("output.json required");
    std::fs::write(path, format!("{output:#}\n")).unwrap();
    println!("{}", serde_json::to_string_pretty(&output["summary"]).unwrap());
    for view in agent.processes.list() {
        if view.running {
            let process = agent.processes.get(&view.id).unwrap();
            process.signal(libc::SIGKILL).unwrap();
            process.wait().await;
        }
    }
    server.abort();
}
