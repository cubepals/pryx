// SPDX-License-Identifier: FSL-1.1-ALv2
//! Data-copy barriers preserve block files and resume only with current authority. Callers
//! hold the workload lock in an owned task through copying and cold resume, including errors.
//! A VM asleep with its memory has no monitor writing its disk: it is copied where it lies and
//! stays asleep. This does not freeze processes; ordinary file backends keep their path.
use super::*;

struct Reservation {
    state: Arc<Mutex<State>>,
    id: WorkloadId,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.state.lock().unwrap().copying.remove(&self.id);
    }
}

impl Manager {
    pub(super) async fn copy_data<T>(
        &self,
        record: &WorkloadRecord,
        running: bool,
        operation: impl std::future::Future<Output = Result<T, NodeError>>,
    ) -> Result<T, NodeError> {
        let runtime = self.record_runtime(record)?;
        if !runtime.requires_stopped_data_copy() {
            return operation.await;
        }
        // Stopping a sleeping guest would end its memory, and starting it again would leave it
        // running while its suspension says asleep.
        let asleep = running
            && self.suspension(&record.id).suspended()
            && self.observe(record).await?.is_some_and(|info| info.status == ContainerStatus::Paused);
        let running = running && !asleep;
        let _reservation = if running {
            self.state.lock().unwrap().copying.insert(record.id.clone());
            Some(Reservation { state: self.state.clone(), id: record.id.clone() })
        } else {
            None
        };
        let mut released = Ok(());
        let result = async {
            if running {
                self.note(
                    runtime
                        .stop(
                            &record.container_name,
                            record.spec.stop.signal.as_str(),
                            Duration::from_secs(u64::from(record.spec.stop.timeout_seconds)),
                        )
                        .await,
                )?;
            }
            let lease =
                self.note(runtime.data_copy_lease(record.id.as_str(), &self.store.data_dir(&record.id)).await)?;
            let copied = operation.await;
            released = self.note(lease.release().await);
            copied
        }
        .await;
        if let Err(problem) = released {
            if running {
                self.set_issue(
                    &record.id,
                    Issue::new(
                        IssueCode::RuntimeUnavailable,
                        format!("Cannot confirm data copy lease release, so compute stays stopped ({problem})"),
                    ),
                );
            }
            return result.and(Err(problem.into()));
        }
        if running {
            let resume = async {
                if !matches!(self.lease(), Lease::NotFleet | Lease::Held) || self.fencing(record) {
                    return Err(NodeError::RuntimeUnavailable(
                        "Data copy ended; compute stays stopped because its execution authority expired or was fenced"
                            .into(),
                    ));
                }
                self.require_store_lease(record)?;
                let mut reservation = self.reserve_start(&record.id, u64::from(record.spec.resources.memory_mb))?;
                reservation.launching();
                let start = self.note(self.start_compute(runtime.as_ref(), record).await);
                let observation = self.observe(record).await;
                if observation.is_ok() {
                    reservation.observed();
                }
                start?;
                observation?;
                Ok(())
            }
            .await;
            if let Err(problem) = resume {
                self.set_issue(
                    &record.id,
                    Issue::new(IssueCode::RuntimeUnavailable, format!("Data copy did not resume compute: {problem}")),
                );
                return result.and(Err(problem));
            }
        }
        result
    }
}
