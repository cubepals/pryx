// SPDX-License-Identifier: FSL-1.1-ALv2
//! Workload tier identity and backend-specific policy. Capacity remains in the single manager;
//! the router owns dispatch, while these checks never let an ensure change a recorded tier.
use super::*;
use crate::protocol::Isolation;

impl Manager {
    pub(super) fn note_tier<T>(&self, tier: &str, result: Result<T, RuntimeError>) -> Result<T, RuntimeError> {
        if result.as_ref().is_err_and(RuntimeError::is_unavailable) {
            self.runtime.tier_unavailable(tier);
        }
        self.note(result)
    }
    pub(super) async fn validate_resources(
        &self,
        runtime: &dyn ContainerRuntime,
        resources: &crate::protocol::Resources,
    ) -> Result<(), NodeError> {
        if let Some(limits) = self.note_tier(runtime.isolation(), runtime.resource_limits().await)? {
            limits.validate(resources).map_err(NodeError::Invalid)?;
        }
        Ok(())
    }
    pub(super) fn runtime_for(&self, tier: Option<Isolation>) -> Result<Arc<dyn ContainerRuntime>, NodeError> {
        let name = tier.map(Isolation::as_str).unwrap_or(self.runtime.isolation());
        if let Some(backend) = self.runtime.backend(name) {
            return Ok(backend);
        }
        if !self.runtime.multi_tier() && name == self.runtime.isolation() {
            return Ok(self.runtime.clone());
        }
        Err(NodeError::Invalid(vec![FieldError {
            field: "isolation".into(),
            problem: format!("this host does not serve the {name} tier"),
        }]))
    }
    pub(super) fn record_tier(&self, record: &WorkloadRecord) -> &'static str {
        record
            .isolation
            .map(Isolation::as_str)
            .unwrap_or_else(|| if self.runtime.multi_tier() { "unresolved" } else { self.runtime.isolation() })
    }
    pub(super) fn record_runtime(&self, record: &WorkloadRecord) -> Result<Arc<dyn ContainerRuntime>, NodeError> {
        if record.isolation.is_none() && self.runtime.multi_tier() {
            return Err(NodeError::Invalid(vec![FieldError {
                field: "isolation".into(),
                problem: "this legacy workload's tier is unresolved; use its previous explicit runtime configuration once before enabling auto".into(),
            }]));
        }
        self.runtime_for(record.isolation)
    }
    pub(super) fn spec_runtime(
        &self,
        id: &WorkloadId,
        spec: &WorkloadSpec,
    ) -> Result<Arc<dyn ContainerRuntime>, NodeError> {
        let previous = self.state.lock().unwrap().records.get(id).cloned();
        if let Some(record) = previous {
            let tier = self.record_tier(&record);
            if spec.isolation.is_some_and(|requested| requested.as_str() != tier) {
                return Err(NodeError::Invalid(vec![FieldError {
                    field: "isolation".into(),
                    problem: format!("this workload keeps its {tier} tier for its lifetime"),
                }]));
            }
            return self.record_runtime(&record);
        }
        self.runtime_for(spec.isolation)
    }
    pub(super) fn bind_record(&self, record: &WorkloadRecord) -> Result<(), NodeError> {
        Ok(self.runtime.bind(&record.container_name, record.container_id.as_deref(), self.record_tier(record))?)
    }
    pub(super) fn record_data_owner(&self, record: &WorkloadRecord) -> Result<(u32, u32), NodeError> {
        if self.record_tier(record) == "container" && self.config.workloads.data_owner.is_some() {
            return Ok(self.config.data_owner_ids());
        }
        Ok(self.record_runtime(record)?.data_owner(record.id.as_str())?.unwrap_or_else(|| self.config.data_owner_ids()))
    }
    pub(super) fn uses_store_lease(&self, record: &WorkloadRecord) -> bool {
        self.config.store_lease.enabled && self.record_tier(record) == "container"
    }
}
