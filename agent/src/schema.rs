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
    generator.subschema_for::<crate::process::View>();
    generator.subschema_for::<crate::process::ResultView>();
    generator.subschema_for::<crate::process::Event>();
    generator.subschema_for::<crate::ports::Exposed>();
    generator.subschema_for::<crate::paths::FileInfo>();
    generator.subschema_for::<crate::logbook::Frame>();
    let mut definitions = generator.take_definitions(true);
    definitions.extend(requests.take_definitions(true));
    let mut document = serde_json::json!({
        "openapi":"3.1.0", "info":{"title":"pryx sandbox agent", "version":env!("CARGO_PKG_VERSION")},
        "x-pryx-agent-version":crate::contract::API_VERSION,
        "paths":{}, "components":{"schemas":definitions}
    });
    for (method, path, schema) in [
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
    ] {
        let mut operation = serde_json::json!({"x-pryx-agent-version":crate::contract::API_VERSION,"responses":{"200":{"description":"Completed"},"204":{"description":"Completed without content"}}});
        if let Some(schema) = schema {
            operation["requestBody"] = serde_json::json!({"required":true,"content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{schema}")}}}});
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
