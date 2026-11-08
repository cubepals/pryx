// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Stateful language kernels use the maintained Jupyter client and the existing process owner.
//! No fleet or lifecycle code lives here, and no caller supplies a kernel executable.
//!
//! Parts (`kernels/`):
//! - `events.rs`: agent wire types and lossless Jupyter MIME/event projection.
//! - `runtime.rs`: supervised kernel startup, channels and bounded execution.
//! - `registry.rs`: named contexts and independent lifecycle control.
//! - `api.rs`: authenticated kernel routes and sandbox-local MCP results.

mod api;
pub mod events;
mod registry;
mod runtime;

#[cfg(feature = "mcp")]
pub(crate) use api::mcp;
pub(crate) use api::routes;

/// Kernel routes evolve independently of the existing agent convenience routes.
pub const VERSION: u16 = 1;
pub const PRESTART_ENV: &str = "PRYX_KERNEL_PRESTART";
#[cfg(feature = "schema")]
pub(crate) use runtime::DEFAULT_RUN;

pub(crate) async fn prestart(agent: &crate::context::Agent) -> Result<(), crate::context::Failure> {
    let Ok(languages) = std::env::var(PRESTART_ENV) else { return Ok(()) };
    for name in languages.split(',').filter(|name| !name.is_empty()) {
        let language = match name {
            "python" => events::Language::Python,
            "javascript" => events::Language::Javascript,
            _ => return Err(runtime::failure("invalid_request", "Unknown prestarted kernel language")),
        };
        agent.processes.kernels.open(&agent.processes, language, String::new()).await?;
    }
    Ok(())
}
