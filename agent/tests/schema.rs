// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

#[test]
fn the_committed_agent_schema_matches_serde() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("openapi.json");
    let encoded = format!("{}\n", serde_json::to_string_pretty(&pryx_agent::schema::document()).unwrap());
    if std::env::var_os("PRYX_WRITE_AGENT_SCHEMA").is_some() {
        std::fs::write(path, encoded).unwrap();
    } else {
        assert_eq!(std::fs::read_to_string(path).unwrap_or_default(), encoded);
    }
}
