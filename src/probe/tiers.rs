// SPDX-License-Identifier: FSL-1.1-ALv2
//! Tier prerequisites for the standalone host probe. This never starts a sandbox or changes
//! configuration; configured doctor separately reports which of these tiers was enabled.
use super::{Capability, Report, State};

pub(super) fn read(report: &mut Report) {
    let engines = ["docker", "docker-user", "podman", "podman-user", "engine-env"];
    let container = if engines.iter().any(|name| report.on(name)) {
        Capability::on("A Docker-compatible engine answered info")
    } else {
        let unknown = engines
            .iter()
            .filter_map(|name| report.capabilities.get(*name))
            .any(|capability| capability.state == State::Unknown);
        Capability::new(
            if unknown { State::Unknown } else { State::Off },
            "No accessible Docker-compatible engine answered info",
            "Start an engine and grant this user access to its socket",
        )
    };
    let process = crate::runtime::configured::process_available(report.capabilities.get("separate-users"));
    let mut config = crate::config::WasmConfig::default();
    if let Some(path) = std::env::var_os("PRYX_AGENTOS_DISTRIBUTION") {
        config.distribution = path.into();
    }
    let wasm = crate::runtime::configured::wasm_available(&config);
    report.tiers = std::collections::BTreeMap::from([
        ("container".into(), container),
        ("process".into(), process),
        ("wasm".into(), wasm),
    ]);
}
