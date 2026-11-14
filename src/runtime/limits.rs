// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Engine resource enforcement, independent of admission reservations. An explicit memory
//! opt-in permits only a missing controller; it cannot remove a limit an engine can enforce.
use crate::protocol::{FieldError, Guarantee, Resources};

#[derive(Clone, Copy, Debug)]
pub struct ResourceLimits {
    pub memory: bool,
    pub cpu: bool,
    pub pids: bool,
    pub rootless: bool,
}
impl ResourceLimits {
    pub(crate) fn validate(self, resources: &Resources) -> Result<(), Vec<FieldError>> {
        let mut errors = Vec::new();
        if !self.memory && resources.allow_unenforced_memory != Some(true) {
            errors.push(FieldError { field: "resources.memoryMb".into(), problem: format!(
                "{} is unavailable on this engine; delegate the memory controller or explicitly allow unenforced memory", Guarantee::MemoryLimit.feature()) });
        }
        if !self.cpu {
            for (field, present) in
                [("cpuMillis", resources.cpu_millis.is_some()), ("cpuWeight", resources.cpu_weight.is_some())]
            {
                if present {
                    errors.push(FieldError {
                        field: format!("resources.{field}"),
                        problem: format!(
                            "{} is unavailable on this engine; delegate the cpu controller",
                            Guarantee::CpuLimit.feature()
                        ),
                    });
                }
            }
        }
        if !self.pids && resources.pids_limit.is_some() {
            errors.push(FieldError {
                field: "resources.pidsLimit".into(),
                problem: "pids-limit is unavailable on this engine; delegate the pids controller".into(),
            });
        }
        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(value: serde_json::Value) -> Resources {
        serde_json::from_value(value).unwrap()
    }
    #[test]
    fn reservations_cannot_silently_replace_requested_limits() {
        let limits = ResourceLimits { memory: false, cpu: false, pids: false, rootless: true };
        let mut resources =
            request(serde_json::json!({"memoryMb": 256, "cpuMillis": 100, "cpuWeight": 1024, "pidsLimit": 32}));
        let errors = limits.validate(&resources).unwrap_err();
        assert_eq!(errors.len(), 4);
        assert!(errors.iter().any(|error| error.problem.contains(&Guarantee::MemoryLimit.feature())));
        resources.allow_unenforced_memory = Some(true);
        assert_eq!(
            limits.validate(&resources).unwrap_err().len(),
            3,
            "memory acceptance cannot drop CPU or pids requests"
        );
        let resources =
            request(serde_json::json!({"memoryMb": 256, "allowUnenforcedMemory": true, "cpuReservationMillis": 100}));
        limits.validate(&resources).unwrap();
        ResourceLimits { memory: true, cpu: true, pids: true, rootless: false }.validate(&resources).unwrap();
    }
}
