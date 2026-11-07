// SPDX-License-Identifier: FSL-1.1-ALv2
//! MCP tool definitions and sandbox-local effects. Existing owners enforce process deadlines,
//! idempotency, safe file walks, bounded replay and spare-port authority.
use crate::{
    auth::Claims,
    context::{Agent, Failure},
    process::{Event, Start},
};
use axum::http::StatusCode;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::io::Read;

fn invalid(message: &str) -> Failure {
    Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request", message.into())
}
fn string<'a>(args: &'a Value, key: &str) -> Result<&'a str, Failure> {
    args[key].as_str().ok_or_else(|| invalid("Required string argument is missing"))
}
pub(super) fn catalog() -> Value {
    let mut tools = Vec::new();
    let text = json!({"type":"string"});
    let process = json!({"command":{"oneOf":[{"type":"string"},{"type":"array","items":{"type":"string"},"minItems":1}]},"timeout":{"type":"integer","minimum":1},"cwd":text,"env":{"type":"object","additionalProperties":{"type":"string"}},"key":text});
    for (name, description, properties, required) in [
        ("run", "Run a command with bounded collected output", process.clone(), vec!["command"]),
        ("start", "Start a background process", process, vec!["command"]),
        ("processes", "List supervised processes", json!({}), vec![]),
        ("process_get", "Read a process state", json!({"id":text}), vec!["id"]),
        ("process_wait", "Wait for a process", json!({"id":text}), vec!["id"]),
        (
            "process_signal",
            "Signal a process",
            json!({"id":text,"signal":{"enum":["TERM","KILL","INT"]}}),
            vec!["id", "signal"],
        ),
        ("process_stdin", "Write process stdin", json!({"id":text,"text":text}), vec!["id", "text"]),
        ("read", "Read a UTF-8 file, at most 1 MiB", json!({"path":text}), vec!["path"]),
        ("write", "Atomically write a UTF-8 file", json!({"path":text,"text":text}), vec!["path", "text"]),
        ("list_files", "List a directory", json!({"path":text}), vec!["path"]),
        ("stat", "Read file metadata", json!({"path":text}), vec!["path"]),
        ("mkdir", "Create a directory", json!({"path":text}), vec!["path"]),
        ("delete_file", "Delete a file or empty directory", json!({"path":text}), vec!["path"]),
        (
            "expose",
            "Expose a preallocated spare port",
            json!({"port":{"type":"integer","minimum":1,"maximum":u16::MAX}}),
            vec!["port"],
        ),
        (
            "unexpose",
            "Remove an exposed port",
            json!({"port":{"type":"integer","minimum":1,"maximum":u16::MAX}}),
            vec!["port"],
        ),
        ("ports", "List exposed application ports", json!({}), vec![]),
    ] {
        tools.push(json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}));
    }
    json!({"tools":tools})
}
async fn process(agent: &Agent, claims: &Claims, name: &str, args: &Value) -> Result<Value, Failure> {
    if name == "run" || name == "start" {
        let mut body = args.clone();
        let object = body.as_object_mut().ok_or_else(|| invalid("Expected arguments"))?;
        let key = object
            .remove("key")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let start: Start = serde_json::from_value(body).map_err(|_| invalid("Invalid process options"))?;
        let process = agent.processes.start(start, &key, claims.max_exec_seconds)?;
        if name == "start" {
            return Ok(json!(process.view()));
        }
        let result = process.wait().await;
        let (replay, _) = process.subscribe().await;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        for event in replay {
            if let Event::Output { stream, bytes } = event {
                let buffer = if stream == "stderr" { &mut stderr } else { &mut stdout };
                buffer.extend(STANDARD.decode(bytes).map_err(|_| invalid("Invalid process output"))?);
            }
        }
        let mut answer = json!(result);
        answer["stdout"] = json!(String::from_utf8_lossy(&stdout));
        answer["stderr"] = json!(String::from_utf8_lossy(&stderr));
        return Ok(answer);
    }
    if name == "processes" {
        return Ok(json!({"processes":agent.processes.list()}));
    }
    let process = agent.processes.get(string(args, "id")?)?;
    match name {
        "process_get" => Ok(json!(process.view())),
        "process_wait" => Ok(json!(process.wait().await)),
        "process_stdin" => {
            process.write(string(args, "text")?.as_bytes()).await?;
            Ok(Value::Null)
        }
        "process_signal" => {
            let signal = match string(args, "signal")? {
                "TERM" => libc::SIGTERM,
                "KILL" => libc::SIGKILL,
                "INT" => libc::SIGINT,
                _ => return Err(invalid("Unsupported signal")),
            };
            process.signal(signal)?;
            Ok(Value::Null)
        }
        _ => Err(invalid("Unknown process tool")),
    }
}
pub(super) async fn call(agent: &Agent, claims: &Claims, params: &Value) -> Result<Value, Failure> {
    let name = string(params, "name")?;
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if matches!(
        name,
        "run" | "start" | "processes" | "process_get" | "process_wait" | "process_stdin" | "process_signal"
    ) {
        return process(agent, claims, name, &args).await;
    }
    match name {
        "ports" => Ok(json!({"ports":agent.ports.list().await})),
        "expose" | "unexpose" => {
            let port = args["port"]
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .filter(|p| *p != 0)
                .ok_or_else(|| invalid("Invalid port"))?;
            if name == "expose" {
                Ok(json!(agent.ports.expose(port, claims.max_ports).await?))
            } else {
                agent.ports.unexpose(port).await?;
                Ok(Value::Null)
            }
        }
        _ => files(agent, name, &args),
    }
}
fn files(agent: &Agent, name: &str, args: &Value) -> Result<Value, Failure> {
    let path = string(args, "path")?;
    match name {
        "read" => {
            let file = agent.data.read(path)?;
            if file.metadata()?.len() > crate::contract::CAPTURE_BYTES as u64 {
                return Err(invalid("MCP file exceeds 1 MiB"));
            }
            let mut bytes = Vec::new();
            file.take(crate::contract::CAPTURE_BYTES as u64 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > crate::contract::CAPTURE_BYTES {
                return Err(invalid("MCP file exceeds 1 MiB"));
            }
            Ok(json!({"text":String::from_utf8(bytes).map_err(|_| invalid("File is not UTF-8"))?}))
        }
        "write" => {
            agent.data.write_atomic(path, string(args, "text")?.as_bytes(), true)?;
            Ok(Value::Null)
        }
        "list_files" => Ok(json!({"files":agent.data.list(path)?})),
        "stat" => Ok(json!(agent.data.stat(path)?)),
        "mkdir" => {
            agent.data.mkdir(path)?;
            Ok(Value::Null)
        }
        "delete_file" => {
            agent.data.delete(path)?;
            Ok(Value::Null)
        }
        _ => Err(invalid("Unknown tool")),
    }
}
