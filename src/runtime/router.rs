// SPDX-License-Identifier: FSL-1.1-ALv2
//! Routes owned workload identities to existing backends. It never retries a mutation on another
//! tier. Backend implementations retain their own journals, execution and confinement policies.
use super::*;
use futures_util::{StreamExt, stream};
use std::sync::{Arc, Mutex};

pub struct Router {
    backends: Vec<Arc<dyn ContainerRuntime>>,
    multiple: bool,
    routes: Mutex<BTreeMap<String, String>>,
    info: Mutex<BTreeMap<String, RuntimeInfo>>,
    ready: Mutex<std::collections::BTreeSet<String>>,
    eras: Mutex<BTreeMap<String, u64>>,
}
pub(crate) fn for_manager(runtime: Arc<dyn ContainerRuntime>) -> Arc<dyn ContainerRuntime> {
    if runtime.multi_tier() || runtime.backend(runtime.isolation()).is_some() {
        return runtime;
    }
    let mut router = Router::new(vec![runtime]).expect("one runtime is a nonempty tier set");
    router.multiple = false;
    Arc::new(router)
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
        Ok(Self {
            backends,
            multiple: true,
            routes: Mutex::new(BTreeMap::new()),
            info: Mutex::new(BTreeMap::new()),
            ready: Mutex::new(std::collections::BTreeSet::new()),
            eras: Mutex::new(BTreeMap::new()),
        })
    }
    fn reported<T>(&self, tier: &str, result: Result<T, RuntimeError>) -> Result<T, RuntimeError> {
        if result.as_ref().is_err_and(RuntimeError::is_unavailable) {
            self.unavailable(tier);
        }
        result
    }
    fn unavailable(&self, tier: &str) {
        *self.eras.lock().unwrap().entry(tier.into()).or_default() += 1;
        self.ready.lock().unwrap().remove(tier);
        self.info.lock().unwrap().remove(tier);
    }
    async fn read_info(&self, all: bool) -> Result<RuntimeInfo, RuntimeError> {
        if !self.multiple {
            return if all { self.default().info().await } else { self.default().serving_info().await };
        }
        let mut first = None;
        let mut failure = None;
        for backend in &self.backends {
            let health = async { if all { backend.info().await } else { backend.serving_info().await } };
            let result = tokio::time::timeout(engine::PROBE_TIMEOUT, health)
                .await
                .unwrap_or_else(|_| Err(RuntimeError::Unavailable("health check timed out".into())));
            match result {
                Ok(info) => {
                    self.info.lock().unwrap().insert(backend.isolation().into(), info.clone());
                    if first.is_none() {
                        first = Some(info);
                    }
                }
                Err(error) => {
                    self.unavailable(backend.isolation());
                    failure = Some(RuntimeError::Unavailable(format!("{} tier: {error}", backend.isolation())));
                }
            }
        }
        if all && let Some(error) = failure {
            return Err(error);
        }
        first.ok_or_else(|| RuntimeError::Unavailable("all tiers are unavailable".into()))
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
    fn remember<'a>(
        &self,
        discovered: impl IntoIterator<Item = (&'a str, &'a ContainerInfo)>,
    ) -> Result<(), RuntimeError> {
        let mut routes = self.routes.lock().unwrap();
        let mut pending = BTreeMap::new();
        for (tier, info) in discovered {
            for identity in [&info.name, &info.id] {
                if routes.get(identity).is_some_and(|previous| previous != tier)
                    || pending.get(identity).is_some_and(|previous| *previous != tier)
                {
                    return Err(RuntimeError::Conflict(format!("ambiguous runtime identity {identity}")));
                }
                pending.insert(identity, tier);
            }
        }
        // Discovery is read-only until the complete result is consistent with owned routes.
        routes.extend(pending.into_iter().map(|(identity, tier)| (identity.clone(), tier.into())));
        Ok(())
    }
}

#[async_trait]
impl ContainerRuntime for Router {
    fn multi_tier(&self) -> bool {
        self.multiple
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
    fn bound_isolation(&self, name: &str) -> Option<&'static str> {
        let tier = self.routes.lock().unwrap().get(name).cloned()?;
        self.backends.iter().find(|backend| backend.isolation() == tier).map(|backend| backend.isolation())
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
        self.read_info(true).await
    }
    async fn serving_info(&self) -> Result<RuntimeInfo, RuntimeError> {
        self.read_info(false).await
    }
    fn tier_ready(&self, tier: &str) -> bool {
        !self.multiple || (self.ready.lock().unwrap().contains(tier) && self.info.lock().unwrap().contains_key(tier))
    }
    fn tier_era(&self, tier: &str) -> u64 {
        self.eras.lock().unwrap().get(tier).copied().unwrap_or_default()
    }
    fn tier_unavailable(&self, tier: &str) {
        self.unavailable(tier);
    }
    async fn inventory(&self, labels: &[String]) -> Result<Inventory, RuntimeError> {
        if !self.multiple {
            return Ok(Inventory { containers: self.list(labels).await?, unavailable: BTreeMap::new() });
        }
        let mut all = Vec::new();
        let mut unavailable = BTreeMap::new();
        let mut complete = Vec::new();
        for tier in self.routes.lock().unwrap().values() {
            if self.backend(tier).is_none() {
                unavailable.insert(tier.clone(), format!("the recorded {tier} tier is unavailable"));
            }
        }
        for backend in &self.backends {
            let era = self.tier_era(backend.isolation());
            match backend.list(labels).await {
                Ok(containers) => {
                    all.extend(containers.into_iter().map(|info| (backend.isolation(), info)));
                    complete.push((backend.isolation(), era));
                }
                Err(error) if error.is_unavailable() => {
                    self.unavailable(backend.isolation());
                    unavailable.insert(backend.isolation().into(), error.to_string());
                }
                Err(error) => return Err(error),
            }
        }
        self.remember(all.iter().map(|(tier, info)| (*tier, info)))?;
        let eras = self.eras.lock().unwrap();
        self.ready.lock().unwrap().extend(
            complete
                .into_iter()
                .filter(|(tier, era)| eras.get(*tier).copied().unwrap_or_default() == *era)
                .map(|(tier, _)| String::from(tier)),
        );
        Ok(Inventory { containers: all.into_iter().map(|(_, info)| info).collect(), unavailable })
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
        let id = self.reported(backend.isolation(), backend.create(spec).await)?;
        self.bind(&spec.name, Some(&id), backend.isolation())?;
        Ok(id)
    }
    async fn start(&self, name: &str) -> Result<(), RuntimeError> {
        let backend = self.target(name)?;
        self.reported(backend.isolation(), backend.start(name).await)
    }
    async fn stop(&self, name: &str, signal: &str, grace: Duration) -> Result<(), RuntimeError> {
        let backend = self.target(name)?;
        self.reported(backend.isolation(), backend.stop(name, signal, grace).await)
    }
    async fn kill(&self, name: &str) -> Result<(), RuntimeError> {
        let backend = self.target(name)?;
        self.reported(backend.isolation(), backend.kill(name).await)
    }
    async fn pause(&self, name: &str) -> Result<(), RuntimeError> {
        let backend = self.target(name)?;
        self.reported(backend.isolation(), backend.pause(name).await)
    }
    async fn unpause(&self, name: &str) -> Result<(), RuntimeError> {
        let backend = self.target(name)?;
        self.reported(backend.isolation(), backend.unpause(name).await)
    }
    async fn disable_restart(&self, name: &str) -> Result<(), RuntimeError> {
        let backend = self.target(name)?;
        self.reported(backend.isolation(), backend.disable_restart(name).await)
    }
    async fn remove(&self, name: &str) -> Result<(), RuntimeError> {
        let backend = self.target(name)?;
        self.reported(backend.isolation(), backend.remove(name).await)
    }
    async fn inspect(&self, name: &str) -> Result<Option<ContainerInfo>, RuntimeError> {
        match self.target(name) {
            Ok(backend) => {
                let result = backend.inspect(name).await;
                if result.as_ref().is_err_and(RuntimeError::is_unavailable) {
                    self.unavailable(backend.isolation());
                }
                return result;
            }
            Err(RuntimeError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
        let mut found = None;
        for backend in &self.backends {
            if let Some(info) = backend.inspect(name).await? {
                if found.is_some() {
                    return Err(RuntimeError::Conflict(format!("ambiguous runtime identity {name}")));
                }
                found = Some((backend.isolation(), info));
            }
        }
        if let Some((tier, info)) = found {
            self.remember(std::iter::once((tier, &info)))?;
            Ok(Some(info))
        } else {
            Ok(None)
        }
    }
    async fn list(&self, labels: &[String]) -> Result<Vec<ContainerInfo>, RuntimeError> {
        for tier in self.routes.lock().unwrap().values() {
            if self.backend(tier).is_none() {
                return Err(RuntimeError::Unavailable(format!("the recorded {tier} tier is unavailable")));
            }
        }
        let mut all = Vec::new();
        let mut names = std::collections::BTreeSet::new();
        for backend in &self.backends {
            for info in backend.list(labels).await? {
                if !names.insert(info.name.clone()) {
                    return Err(RuntimeError::Conflict(format!("ambiguous runtime identity {}", info.name)));
                }
                all.push((backend.isolation(), info));
            }
        }
        self.remember(all.iter().map(|(tier, info)| (*tier, info)))?;
        Ok(all.into_iter().map(|(_, info)| info).collect())
    }
    async fn exec(
        &self,
        container: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
    ) -> Result<ExecOutput, RuntimeError> {
        let backend = self.target(container)?;
        self.reported(backend.isolation(), backend.exec(container, command, timeout, max_output).await)
    }
    async fn exec_stream(
        &self,
        container: &str,
        command: &[String],
        timeout: Duration,
        max_output: usize,
        output: tokio::sync::mpsc::Sender<LogChunk>,
    ) -> Result<ExecOutput, RuntimeError> {
        let backend = self.target(container)?;
        self.reported(backend.isolation(), backend.exec_stream(container, command, timeout, max_output, output).await)
    }
    fn logs(&self, name: &str, options: LogOptions) -> BoxStream<'static, Result<LogChunk, RuntimeError>> {
        match self.target(name) {
            Ok(backend) => backend.logs(name, options),
            Err(error) => stream::once(async { Err(error) }).boxed(),
        }
    }
    async fn stats(&self, name: &str) -> Result<Option<RawStats>, RuntimeError> {
        let backend = self.target(name)?;
        self.reported(backend.isolation(), backend.stats(name).await)
    }
    fn events(&self, labels: &[String]) -> BoxStream<'static, Result<RuntimeEvent, RuntimeError>> {
        stream::select_all(self.backends.iter().map(|r| r.events(labels)).collect::<Vec<_>>()).boxed()
    }
}
