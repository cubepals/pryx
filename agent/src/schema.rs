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
        "paths":{}, "components":{"schemas":definitions}
    });
    document.sort_all_objects();
    document
}
