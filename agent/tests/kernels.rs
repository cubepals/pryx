// SPDX-License-Identifier: FSL-1.1-ALv2
//! Projection fixtures were captured from ipykernel 7.4.0 through jupyter_client 8.10.0.
//! They retain real owner messages, including rich MIME bundles and shell error replies.
use pryx_agent::kernels::events::{KernelEvent, project};
use serde_json::{Value, json};

#[test]
fn recorded_kernel_messages_preserve_values_and_errors() {
    let messages: Vec<Value> = serde_json::from_str(include_str!("fixtures/ipykernel-messages.json")).unwrap();
    let events: Vec<Value> = messages
        .into_iter()
        .filter_map(|message| {
            let message: jupyter_protocol::JupyterMessage = serde_json::from_value(message).unwrap();
            project(message.content).unwrap().map(|event| serde_json::to_value(event).unwrap())
        })
        .collect();
    assert!(events.iter().any(|event| event["type"] == "stream"
        && event["stream"] == "stdout"
        && event["text"].as_str().unwrap().contains("recorded stdout")));
    assert!(events.iter().any(|event| event["type"] == "stream" && event["stream"] == "stderr"));
    assert!(events.iter().any(|event| event["type"] == "result"
        && event["data"]["text/plain"] == "4"
        && event["data"]["application/json"] == 4));
    assert!(events.iter().any(|event| event["type"] == "display"
        && event["data"]["application/json"] == json!({"answer":4})
        && event["data"]["text/html"] == "<b>rich</b>"));
    assert!(
        events.iter().any(|event| event["type"] == "display" && event["update"] == true && event["displayId"] == "one")
    );
    assert!(events.iter().any(|event| event["type"] == "error"
        && event["name"] == "ValueError"
        && !event["traceback"].as_array().unwrap().is_empty()));
    assert!(events.iter().any(|event| event["type"] == "done" && event["status"] == "error"));
    assert_eq!(serde_json::to_value(KernelEvent::failure("kernel_died", "exited")).unwrap()["name"], "kernel_died");
}

#[test]
fn recorded_javascript_messages_preserve_rich_results_and_tracebacks() {
    let messages: Vec<Value> = serde_json::from_str(include_str!("fixtures/ijavascript-messages.json")).unwrap();
    let events: Vec<Value> = messages
        .into_iter()
        .filter_map(|message| {
            let message: jupyter_protocol::JupyterMessage = serde_json::from_value(message).unwrap();
            project(message.content).unwrap().map(|event| serde_json::to_value(event).unwrap())
        })
        .collect();
    assert!(events.iter().any(|e| e["type"] == "result" && e["data"]["text/plain"] == "4"));
    assert!(
        events
            .iter()
            .any(|e| e["data"]["application/json"] == json!({"answer":4}) && e["data"]["text/html"] == "<b>rich</b>")
    );
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "error" && e["name"] == "Error" && !e["traceback"].as_array().unwrap().is_empty())
    );
}
