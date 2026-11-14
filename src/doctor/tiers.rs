// SPDX-License-Identifier: FSL-1.1-ALv2
//! Configured tier availability and actionable refusals. It reads prerequisites only;
//! runtime construction and per-workload routing remain in runtime/configured and the manager.
use super::check::Check;
use crate::config::{Config, RuntimeKind};

pub(super) async fn checks(config: &Config) -> Vec<Check> {
    let available = crate::runtime::configured::availability(config).await;
    let mut checks = Vec::new();
    let mut enabled = 0;
    for (tier, name, explicit) in [
        ("container", "tier.container", RuntimeKind::Container),
        ("process", "tier.process", RuntimeKind::Process),
        ("wasm", "tier.wasm", RuntimeKind::Wasm),
    ] {
        if config.runtime != RuntimeKind::Auto && config.runtime != explicit {
            checks.push(Check::warn(
                name,
                "off: this configuration selects exactly one other tier",
                "Set runtime = \"auto\" to enable every supported tier",
            ));
        } else if let Some(reason) = &available[tier] {
            checks.push(Check::warn(
                name,
                format!("off: {reason}"),
                "Inspect pryxd probe for the missing host capability or distribution",
            ));
        } else {
            enabled += 1;
            checks.push(Check::pass(name, "on: this tier's prerequisites are available"));
        }
    }
    if enabled == 0 && config.runtime == RuntimeKind::Auto {
        checks.push(
            Check::fail(
                "tiers",
                "No supported isolation tier is available",
                "Inspect the off reasons above and enable one supported host capability",
            )
            .gating(),
        );
    }
    checks
}
