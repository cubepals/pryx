// SPDX-License-Identifier: FSL-1.1-ALv2
//! Routes owned workload identities to existing backends. It never retries a mutation on another
//! tier. Backend implementations retain their own journals, execution and confinement policies.
use super::*;
use futures_util::{StreamExt, stream};
use std::sync::{Arc, Mutex};

pub struct Router {
    backends: Vec<Arc<dyn ContainerRuntime>>,
    routes: Mutex<BTreeMap<String, String>>,
    info: Mutex<BTreeMap<String, RuntimeInfo>>,
}
impl Router {
    pub fn new(backends: Vec<Arc<dyn ContainerRuntime>>) -> Result<Self, RuntimeError> {
        if backends.is_empty() {
            return Err(RuntimeError::Unavailable("no supported isolation tier is available".into()));
        }
        let tiers: std::collections::BTreeSet<_> = backends.iter().map(|r| r.isolation()).collect();
        if tiers.len() != backends.len() {
            return Err(RuntimeError::Other("duplicate isolation backend".into()));
        }
        Ok(Self { backends, routes: Mutex::new(BTreeMap::new()), info: Mutex::new(BTreeMap::new()) })
    }
    fn default(&self) -> &dyn ContainerRuntime {
        self.backends[0].as_ref()
    }
    fn target(&self, name: &str) -> Result<Arc<dyn ContainerRuntime>, RuntimeError> {
        let tier = self
            .routes
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::NotFound(format!("no owned runtime route for {name}")))?;
        self.backend(&tier).ok_or_else(|| RuntimeError::Unavailable(format!("{tier} tier is unavailable")))
    }
    fn remember(&self, tier: &str, info: &ContainerInfo) -> Result<(), RuntimeError> {
        self.bind(&info.name, Some(&info.id), tier)
    }
}

#[async_trait]
impl ContainerRuntime for Router {
    fn multi_tier(&self) -> bool {
        true
    }
    fn backend(&self, tier: &str) -> Option<Arc<dyn ContainerRuntime>> {
        self.backends.iter().find(|r| r.isolation() == tier).cloned()
    }
    fn tiers(&self) -> Vec<&'static str> {
        self.backends.iter().map(|r| r.isolation()).collect()
    }
    fn tier_info(&self, tier: &str) -> Option<RuntimeInfo> {
        self.info.lock().unwrap().get(tier).cloned()
    }
    fn bind(&self, name: &str, id: Option<&str>, tier: &str) -> Result<(), RuntimeError> {
        let mut routes = self.routes.lock().unwrap();
        if let Some(id) = id {
            if routes.get(id).is_some_and(|previous| previous != tier) {
                return Err(RuntimeError::Conflict(format!("ambiguous runtime identity {id}")));
            }
            routes.insert(id.into(), tier.into());
        }
        routes.insert(name.into(), tier.into());
        Ok(())
    }
    fn data_deleted(&self, id: &str) -> Result<(), RuntimeError> {
        if let Some(process) = self.backend("process") {
            process.data_deleted(id)?;
        }
        Ok(())
    }
    fn isolation(&self) -> &'static str {
        self.default().isolation()
    }
    fn is_process(&self) -> bool {
        self.default().is_process()
    }
    fn is_wasm(&self) -> bool {
        self.default().is_wasm()
    }
    fn supports_agent(&self) -> bool {
        self.default().supports_agent()
    }
    fn supports_native_fuse(&self) -> bool {
        self.default().supports_native_fuse()
    }
    fn supports_hostname_egress(&self) -> bool {
        self.default().supports_hostname_egress()
    }
    fn supports_network_policy(&self) -> bool {
        self.default().supports_network_policy()
    }
    async fn info(&self) -> Result<RuntimeInfo, RuntimeError> {
        let mut first = None;
        let mut failure = None;
        for backend in &self.backends {
            match backend.info().await {
                Ok(info) => {
                    self.info.lock().unwrap().insert(backend.isolation().into(), info.clone());
                    if first.is_none() {
                        first = Some(info);
                    }
                }
                Err(error) => {
                    self.info.lock().unwrap().remove(backend.isolation());
                    failure = Some(RuntimeError::Unavailable(format!("{} tier: {error}", backend.isolation())));
                }
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        first.ok_or_else(|| RuntimeError::Unavailable("all tiers are unavailable".into()))
    }
    async fn ensure_network(
        &self,
        name: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<NetworkReport, RuntimeError> {
        if let Some(container) = self.backend("container") {
            container.ensure_network(name, labels).await
        } else {
            self.default().ensure_network(name, labels).await
        }
    }
    async fn ensure_internal_network(&self, name: &str, labels: &BTreeMap<String, String>) -> Result<(), RuntimeError> {
        if let Some(container) = self.backend("container") {
            container.ensure_internal_network(name, labels).await
        } else {
            Ok(())
        }
    }
    async fn network_isolated(&self, name: &str) -> Result<Option<bool>, RuntimeError> {
        if let Some(container) = self.backend("container") {
            container.network_isolated(name).await
        } else {
            self.default().network_isolated(name).await
        }
    }
    async fn ensure_image(&self, image: &str, timeout: Duration) -> Result<bool, RuntimeError> {
        self.default().ensure_image(image, timeout).await
    }
    async fn create(&self, spec: &ContainerSpec) -> Result<String, RuntimeError> {
        let backend = self.target(&spec.name)?;
        let id = backend.create(spec).await?;
        self.bind(&spec.name, Some(&id), backend.isolation())?;
        Ok(id)
    }
    async fn start(&self, name: &str) -> Result<(), RuntimeError> {
        self.target(name)?.start(name).await
    }
    async fn stop(&self, name: &str, signal: &str, grace: Duration) -> Result<(), RuntimeError> {
        self.target(name)?.stop(name, signal, grace).await
    }
    async fn kill(&self, name: &str) -> Result<(), RuntimeError> {
        self.target(name)?.kill(name).await
    }
    async fn pause(&self, name: &str) -> Result<(), RuntimeError> {
        self.target(name)?.pause(name).await
    }
    async fn unpause(&self, name: &str) -> Result<(), RuntimeError> {
        self.target(name)?.unpause(name).await
    }
    async fn disable_restart(&self, name: &str) -> Result<(), RuntimeError> {
        self.target(name)?.disable_restart(name).await
    }
    async fn remove(&self, name: &str) -> Result<(), RuntimeError> {
        self.target(name)?.remove(name).await
    }
    async fn inspect(&self, name: &str) -> Result<Option<ContainerInfo>, RuntimeError> {
        match self.target(name) {
            Ok(backend) => return backend.inspect(name).await,
            Err(RuntimeError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
        let mut found = None;
        for backend in &self.backends {
            if let Some(info) = backend.inspect(name).await? {
                if found.is_some() {
                    return Err(RuntimeError::Conflict(format!("ambiguous runtime identity {name}")));
                }
                self.remember(backend.isolation(), &info)?;
                found = Some(info);
            }
        }
        Ok(found)
    }
    async fn list(&self, labels: &[String]) -> Result<Vec<ContainerInfo>, RuntimeError> {
        let mut all = Vec::new();
        let mut names = std::collections::BTreeSet::new();
        for backend in &self.backends {
            for info in backend.list(labels).await? {
                if !names.insert(info.name.clone()) {
                    return Err(RuntimeError::Conflict(format!("ambiguous runtime identity {}", info.name)));
                }
                self.remember(backend.isolation(), &info)?;
                all.push(info);
            }
        }
        Ok(all)
    }
    async fn exec(
        &self,
        container: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
    ) -> Result<ExecOutput, RuntimeError> {
        self.target(container)?.exec(container, command, timeout, max_output).await
    }
    async fn exec_stream(
        &self,
        container: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
        output: tokio::sync::mpsc::Sender<LogChunk>,
    ) -> Result<ExecOutput, RuntimeError> {
        self.target(container)?.exec_stream(container, command, timeout, max_output, output).await
    }
    fn logs(&self, name: &str, options: LogOptions) -> BoxStream<'static, Result<LogChunk, RuntimeError>> {
        match self.target(name) {
            Ok(backend) => backend.logs(name, options),
            Err(error) => stream::once(async { Err(error) }).boxed(),
        }
    }
    async fn stats(&self, name: &str) -> Result<Option<RawStats>, RuntimeError> {
        self.target(name)?.stats(name).await
    }
    fn events(&self, labels: &[String]) -> BoxStream<'static, Result<RuntimeEvent, RuntimeError>> {
        stream::select_all(self.backends.iter().map(|r| r.events(labels)).collect::<Vec<_>>()).boxed()
    }
}
