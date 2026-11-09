// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Kernel request/event types and Jupyter projections. MIME bundles are preserved in full,
//! rather than choosing one representation or parsing a textual value back into a number.

use jupyter_protocol::{JupyterMessageContent, Stdio};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Hash, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Language {
    Python,
    Javascript,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct KernelOpen {
    pub language: Language,
    #[serde(default)]
    pub context: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct KernelRun {
    pub code: String,
    pub timeout: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct KernelComplete {
    pub code: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct KernelView {
    pub id: String,
    pub language: Language,
    pub context: String,
    pub process_id: Option<String>,
    pub pid: Option<u32>,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum KernelEvent {
    Stream { stream: String, text: String },
    Result { data: Value, metadata: Map<String, Value>, execution_count: usize },
    Display { data: Value, metadata: Map<String, Value>, display_id: Option<String>, update: bool },
    Error { name: String, value: String, traceback: Vec<String> },
    Done { execution_count: Option<usize>, status: String },
}

impl KernelEvent {
    pub fn failure(name: &str, value: &str) -> Self {
        Self::Error { name: name.into(), value: value.into(), traceback: Vec::new() }
    }
}

pub fn project(content: JupyterMessageContent) -> Result<Option<KernelEvent>, serde_json::Error> {
    Ok(Some(match content {
        JupyterMessageContent::StreamContent(stream) => KernelEvent::Stream {
            stream: match stream.name {
                Stdio::Stdout => "stdout",
                Stdio::Stderr => "stderr",
            }
            .into(),
            text: stream.text,
        },
        JupyterMessageContent::ExecuteResult(result) => KernelEvent::Result {
            data: serde_json::to_value(result.data)?,
            metadata: result.metadata,
            execution_count: result.execution_count.value(),
        },
        JupyterMessageContent::DisplayData(display) => KernelEvent::Display {
            data: serde_json::to_value(display.data)?,
            metadata: display.metadata,
            display_id: display.transient.and_then(|t| t.display_id),
            update: false,
        },
        JupyterMessageContent::UpdateDisplayData(display) => KernelEvent::Display {
            data: serde_json::to_value(display.data)?,
            metadata: display.metadata,
            display_id: display.transient.display_id,
            update: true,
        },
        JupyterMessageContent::ErrorOutput(error) => {
            KernelEvent::Error { name: error.ename, value: error.evalue, traceback: error.traceback }
        }
        JupyterMessageContent::ExecuteReply(reply) => KernelEvent::Done {
            execution_count: Some(reply.execution_count.value()),
            status: serde_json::to_value(reply.status)?.as_str().unwrap_or("error").into(),
        },
        _ => return Ok(None),
    }))
}
