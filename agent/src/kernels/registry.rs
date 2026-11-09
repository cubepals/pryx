// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Named kernels retain state between requests. Startup/restart serializes separately from code;
//! interrupt and close can always reach the process without taking the command mutex.

use super::{
    events::{KernelView, Language},
    runtime::{Session, failure},
};
use crate::kernel_slots::KernelSlots as Registry;
use crate::{context::Failure, process::Processes};
use std::{
    sync::atomic::Ordering,
    sync::{Arc, RwLock},
};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

const MAX_CONTEXTS: usize = 16;

pub(crate) struct Kernel {
    pub(crate) id: String,
    language: Language,
    context: String,
    transition: AsyncMutex<()>,
    running: RwLock<Option<Arc<Session>>>,
    closed: CancellationToken,
}

impl Registry {
    pub(crate) async fn open(
        &self,
        owner: &Processes,
        language: Language,
        context: String,
    ) -> Result<Arc<Kernel>, Failure> {
        if let Some(tier) = owner.kernel_unsupported_tier() {
            return Err(Failure(
                axum::http::StatusCode::NOT_IMPLEMENTED,
                "unsupported",
                format!(
                    "Language kernels require an isolated native interpreter template; the {tier} tier is unsupported"
                ),
            ));
        }
        if context.len() > 64 || !context.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')) {
            return Err(failure(
                "invalid_request",
                "Context must be at most 64 ASCII letters, digits, hyphens or underscores",
            ));
        }
        let kernel = {
            let mut held = self.0.lock().unwrap();
            let key = (format!("{language:?}"), context.clone());
            if let Some(kernel) = held.get(&key) {
                kernel.clone().downcast::<Kernel>().expect("KernelSlots stores only kernels")
            } else {
                if held.len() >= MAX_CONTEXTS {
                    return Err(failure("kernel_capacity", "Kernel context capacity reached"));
                }
                let kernel = Arc::new(Kernel {
                    id: uuid::Uuid::new_v4().to_string(),
                    language,
                    context,
                    transition: AsyncMutex::new(()),
                    running: RwLock::new(None),
                    closed: CancellationToken::new(),
                });
                held.insert(key, kernel.clone());
                kernel
            }
        };
        let _transition = kernel.transition.lock().await;
        if kernel.closed.is_cancelled() {
            return Err(failure("kernel_closed", "Kernel is closing"));
        }
        if kernel.running.read().unwrap().is_none() {
            let started = tokio::select! {
                result = Session::start(owner, language) => result,
                _ = kernel.closed.cancelled() => return Err(failure("kernel_closed", "Kernel was closed during startup")),
            };
            let session = match started {
                Ok(session) => session,
                Err(error) => {
                    // A context whose first start failed holds no process and would otherwise keep
                    // one of MAX_CONTEXTS slots; concurrent openers waiting on it fail as closed.
                    kernel.closed.cancel();
                    self.0
                        .lock()
                        .unwrap()
                        .retain(|_, value| value.downcast_ref::<Kernel>().is_none_or(|value| value.id != kernel.id));
                    return Err(error);
                }
            };
            *kernel.running.write().unwrap() = Some(session);
        }
        drop(_transition);
        Ok(kernel)
    }

    pub(crate) fn get(&self, id: &str) -> Result<Arc<Kernel>, Failure> {
        self.0
            .lock()
            .unwrap()
            .values()
            .map(|kernel| kernel.clone().downcast::<Kernel>().expect("KernelSlots stores only kernels"))
            .find(|kernel| kernel.id == id)
            .ok_or_else(|| failure("not_found", "No such kernel"))
    }

    pub(crate) fn list(&self) -> Vec<KernelView> {
        self.0
            .lock()
            .unwrap()
            .values()
            .map(|kernel| kernel.downcast_ref::<Kernel>().expect("KernelSlots stores only kernels").view())
            .collect()
    }

    pub(crate) async fn close(&self, id: &str) -> Result<(), Failure> {
        let kernel = match self.get(id) {
            Ok(kernel) => kernel,
            Err(_) => return Ok(()),
        };
        kernel.closed.cancel();
        let _transition = kernel.transition.lock().await;
        let session = kernel.running.read().unwrap().clone();
        if let Some(session) = session {
            session.stop().await?;
        }
        self.0
            .lock()
            .unwrap()
            .retain(|_, value| value.downcast_ref::<Kernel>().is_none_or(|value| value.id != kernel.id));
        Ok(())
    }
}

impl Kernel {
    pub(crate) fn session(&self) -> Result<Arc<Session>, Failure> {
        if self.closed.is_cancelled() {
            return Err(failure("kernel_closed", "Kernel is closing"));
        }
        self.running.read().unwrap().clone().ok_or_else(|| failure("kernel_starting", "Kernel is starting"))
    }

    pub(crate) fn view(&self) -> KernelView {
        let session = self.running.read().unwrap().clone();
        let process = session.as_ref().map(|s| s.process.view());
        let state = match (&session, &process) {
            (_, Some(process)) if !process.running => "dead",
            (Some(session), _) if session.cancel.is_cancelled() => "stopping",
            (Some(session), _) if session.busy.load(Ordering::Acquire) => "busy",
            (Some(_), _) => "idle",
            _ => "starting",
        };
        KernelView {
            id: self.id.clone(),
            language: self.language,
            context: self.context.clone(),
            process_id: process.as_ref().map(|p| p.id.clone()),
            pid: process.map(|p| p.pid),
            state: state.into(),
        }
    }

    pub(crate) async fn restart(&self, owner: &Processes) -> Result<KernelView, Failure> {
        let _transition = self.transition.lock().await;
        if self.closed.is_cancelled() {
            return Err(failure("kernel_closed", "Kernel is closing"));
        }
        let previous = self.running.read().unwrap().clone();
        if let Some(previous) = previous {
            previous.stop().await?;
        }
        let session = tokio::select! {
            result = Session::start(owner, self.language) => result?,
            _ = self.closed.cancelled() => return Err(failure("kernel_closed", "Kernel was closed during restart")),
        };
        *self.running.write().unwrap() = Some(session);
        Ok(self.view())
    }
}
