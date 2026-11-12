// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The agent's serde types own its wire schema. This module is absent from release binaries.

pub fn document() -> serde_json::Value {
    let mut generator = schemars::generate::SchemaSettings::draft2020_12()
        .with(|settings| {
            settings.definitions_path = "/components/schemas".into();
            settings.meta_schema = None;
            settings.contract = schemars::generate::Contract::Serialize;
        })
        .into_generator();
    let mut requests = schemars::generate::SchemaSettings::draft2020_12()
        .with(|settings| {
            settings.definitions_path = "/components/schemas".into();
            settings.meta_schema = None;
            settings.contract = schemars::generate::Contract::Deserialize;
        })
        .into_generator();
    requests.subschema_for::<crate::process::Start>();
    requests.subschema_for::<crate::field_process::Wait>();
    requests.subschema_for::<crate::shells::Open>();
    requests.subschema_for::<crate::shells::Run>();
    requests.subschema_for::<crate::file_search::Search>();
    requests.subschema_for::<crate::file_search::Path>();
    requests.subschema_for::<crate::file_search::Mode>();
    requests.subschema_for::<crate::kernels::events::KernelOpen>();
    requests.subschema_for::<crate::kernels::events::KernelRun>();
    requests.subschema_for::<crate::kernels::events::KernelComplete>();
    generator.subschema_for::<crate::process::View>();
    generator.subschema_for::<crate::process::ResultView>();
    generator.subschema_for::<crate::process::Event>();
    generator.subschema_for::<crate::ports::Exposed>();
    generator.subschema_for::<crate::paths::FileInfo>();
    generator.subschema_for::<crate::logbook::Frame>();
    generator.subschema_for::<crate::kernels::events::KernelView>();
    generator.subschema_for::<crate::kernels::events::KernelEvent>();
    requests.subschema_for::<crate::tools::types::Point>();
    requests.subschema_for::<crate::tools::types::Text>();
    requests.subschema_for::<crate::tools::types::Key>();
    requests.subschema_for::<crate::tools::types::Scroll>();
    requests.subschema_for::<crate::tools::types::BrowserShot>();
    requests.subschema_for::<crate::tools::types::SshExpiry>();
    requests.subschema_for::<crate::tools::types::MountsInit>();
    generator.subschema_for::<crate::tools::types::ToolAccess>();
    generator.subschema_for::<crate::tools::types::SshCreated>();
    generator.subschema_for::<crate::tools::types::SshView>();
    generator.subschema_for::<crate::tools::types::MountView>();
    let mut definitions = generator.take_definitions(true);
    definitions.extend(requests.take_definitions(true));
    let mut document = serde_json::json!({
        "openapi":"3.1.0", "info":{"title":"pryx sandbox agent", "version":env!("CARGO_PKG_VERSION")},
        "x-pryx-agent-version":crate::contract::API_VERSION,
        "x-pryx-kernels-version":crate::kernels::VERSION,
        "x-pryx-kernels":{"version":crate::kernels::VERSION,"prestartEnv":crate::kernels::PRESTART_ENV,"defaultTimeoutSeconds":crate::kernels::DEFAULT_RUN.as_secs()},
        "x-pryx-tools": {"version":crate::tools::defaults::VERSION,"kindEnv":crate::tools::defaults::KIND_ENV,"directoryEnv":crate::tools::defaults::DIRECTORY_ENV,"desktopPort":crate::tools::defaults::DESKTOP_PORT,"browserPort":crate::tools::defaults::BROWSER_PORT,"cdpPort":crate::tools::defaults::CDP_PORT,"vncPort":crate::tools::defaults::VNC_PORT,"forwardHeader":crate::contract::FORWARD_HEADER,"sshForwardHeader":crate::contract::SSH_FORWARD_HEADER,"fuseFeature":crate::contract::FUSE_FEATURE,"mountsEnv":crate::contract::MOUNTS_ENV},
        "paths":{}, "components":{"schemas":definitions}
    });
    for (method, path, schema) in [
        ("post", "/_pryx/desktop", None),
        ("post", "/_pryx/desktop/screenshot", None),
        ("post", "/_pryx/desktop/click", Some("Point")),
        ("post", "/_pryx/desktop/move", Some("Point")),
        ("post", "/_pryx/desktop/type", Some("Text")),
        ("post", "/_pryx/desktop/key", Some("Key")),
        ("post", "/_pryx/desktop/scroll", Some("Scroll")),
        ("post", "/_pryx/browser", None),
        ("post", "/_pryx/browser/screenshot", Some("BrowserShot")),
        ("post", "/_pryx/ssh", None),
        ("delete", "/_pryx/ssh", None),
        ("get", "/_pryx/ssh/{id}", None),
        ("delete", "/_pryx/ssh/{id}", None),
        ("post", "/_pryx/ssh/{id}/expiry", Some("SshExpiry")),
        ("get", "/_pryx/ssh/{id}/connect", None),
        ("get", "/_pryx/mounts", None),
        ("post", "/_pryx/mounts", Some("MountsInit")),
        ("post", "/_pryx/processes/{id}/ready", Some("Wait")),
        ("post", "/_pryx/processes/kill-all", None),
        ("post", "/_pryx/shells", Some("Open")),
        ("post", "/_pryx/shells/{id}/run", Some("Run")),
        ("delete", "/_pryx/shells/{id}", None),
        ("post", "/_pryx/files/find", Some("Search")),
        ("post", "/_pryx/files/grep", Some("Search")),
        ("post", "/_pryx/files/replace", Some("Search")),
        ("post", "/_pryx/files/chmod", Some("Mode")),
        ("get", "/_pryx/files/archive", None),
        ("put", "/_pryx/files/archive", None),
        ("get", "/_pryx/kernels", None),
        ("post", "/_pryx/kernels", Some("KernelOpen")),
        ("delete", "/_pryx/kernels/{id}", None),
        ("post", "/_pryx/kernels/{id}/execute", Some("KernelRun")),
        ("post", "/_pryx/kernels/{id}/interrupt", None),
        ("post", "/_pryx/kernels/{id}/restart", None),
        ("post", "/_pryx/kernels/{id}/complete", Some("KernelComplete")),
    ] {
        let mut operation = serde_json::json!({"x-pryx-agent-version":crate::contract::API_VERSION,"responses":{"200":{"description":"Completed"},"204":{"description":"Completed without content"}}});
        if let Some(schema) = schema {
            operation["requestBody"] = serde_json::json!({"required":true,"content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{schema}")}}}});
        }
        if path == "/_pryx/ssh/{id}/connect" {
            operation["responses"] = serde_json::json!({"101":{"description":"Binary OpenSSH stream over an authenticated WebSocket; lease expiry and revocation close the stream"}});
        }
        if path.starts_with("/_pryx/kernels") {
            operation["x-pryx-kernels-version"] = serde_json::json!(crate::kernels::VERSION);
            if path.ends_with("/execute") {
                operation["responses"]["200"] = serde_json::json!({"description":"One kernel event per NDJSON line, through done", "content":{"application/x-ndjson":{"schema":{"$ref":"#/components/schemas/KernelEvent"}}}});
            }
        }
        let mut parameters = Vec::new();
        if path.contains("{id}") {
            parameters.push(serde_json::json!({"name":"id","in":"path","required":true,"schema":{"type":"string"}}));
        }
        if path.ends_with("/archive") {
            parameters.push(serde_json::json!({"name":"path","in":"query","required":true,"schema":{"type":"string"}}));
            if method == "put" {
                operation["requestBody"] = serde_json::json!({"required":true,"content":{"application/x-tar":{"schema":{"type":"string","format":"binary"}}}});
            }
        }
        if !parameters.is_empty() {
            operation["parameters"] = serde_json::Value::Array(parameters);
        }
        document["paths"][path][method] = operation;
    }
    document.sort_all_objects();
    document
}
