// SPDX-License-Identifier: FSL-1.1-ALv2
//! Older process sandboxes can outlive their supervisor under a shared UID. A lost ancestry is
//! never proof of cleanup: uncertainty survives record removal until the kernel's boot identity
//! changes. Normal stop clears a generation only with a signed final descendant-retirement
//! checkpoint or an empty dedicated UID. Not the program set of a policed sandbox (`network.rs`).
use super::{Entry, ProcessRuntime, RuntimeError, journal, observe, resources};

// pid_namespaces(7): the first process in a new PID namespace has PID 1.
const NAMESPACED_AGENT_PID: u32 = 1;

impl ProcessRuntime {
    pub(super) fn legacy_boot_uncertain(&self) -> bool {
        journal::network_boot_uncertain(&self.state.root, crate::host::boot_id().as_deref()).unwrap_or(true)
    }

    /// Whether this generation may have left descendants that no live supervisor identifies.
    pub(super) fn legacy_pending(&self, entry: &Entry) -> bool {
        if entry.network.is_some() {
            return false;
        }
        if entry.network_boot.as_deref().is_some_and(|before| {
            before != journal::UNKNOWN_BOOT && crate::host::boot_id().is_some_and(|now| now != before)
        }) {
            return false;
        }
        // A pre-exec child may not have dropped to its dedicated UID yet. Even an empty UID
        // cannot retire this intent until an actual PID checkpoint or a proven new boot.
        if entry.network_launch_pending {
            return true;
        }
        if entry.network_clean || (entry.pid.is_none() && entry.started.is_none() && entry.network_boot.is_none()) {
            return false;
        }
        if !self.shared_uid()
            && entry.spec.agent.as_ref().is_some_and(|agent| {
                self.state
                    .owners
                    .lock()
                    .unwrap()
                    .get(&agent.sandbox)
                    .is_some_and(|(uid, _)| observe::uid_processes_proven(*uid).is_ok_and(|tasks| tasks.is_empty()))
            })
        {
            return false;
        }
        !self.agent_retirement_proven(entry)
    }

    fn agent_retirement_proven(&self, entry: &Entry) -> bool {
        let Some(nonce) = &entry.network_stop_nonce else { return false };
        let Some(agent) = &entry.spec.agent else { return false };
        let Some(pid) = entry.pid.map(|pid| if entry.namespaced.is_some() { NAMESPACED_AGENT_PID } else { pid }) else {
            return false;
        };
        let Some(key) = entry
            .spec
            .env
            .iter()
            .find(|(name, _)| name == &agent.secret_env)
            .and_then(|(_, value)| pryx_agent::auth::SigningKey::from_encoded(value).ok())
        else {
            return false;
        };
        let Ok(data) = pryx_agent::paths::DataDir::open(&entry.spec.data_dir) else { return false };
        let Ok(proof) = pryx_agent::state::read::<pryx_agent::process_retirement::Proof>(
            &data,
            pryx_agent::process_retirement::CHECKPOINT,
        ) else {
            return false;
        };
        pryx_agent::process_retirement::verify(&proof, &key.public_key(), nonce, &agent.sandbox, agent.epoch, pid)
    }

    /// Called before deleting a legacy record too: deletion cannot erase unattributed compute.
    pub(super) fn note_legacy_loss(&self, entry: &mut Entry) -> Result<(), RuntimeError> {
        if entry.network.is_some() || observe::matching_pid(entry.pid, entry.ticks).is_some() {
            return Ok(());
        }
        if self.legacy_pending(entry) {
            journal::note_network_uncertain(&self.state.root)?;
        } else {
            entry.network_clean = true;
        }
        Ok(())
    }

    pub(super) fn legacy_history(&self) -> Result<(), RuntimeError> {
        for name in self.names() {
            let mut entry = self.entry(&name)?;
            if entry.network.is_some() {
                continue;
            }
            let before = (entry.network_boot.clone(), entry.network_clean);
            if entry.network_boot.is_none() && (entry.pid.is_some() || entry.started.is_some()) {
                entry.network_boot = Some(crate::host::boot_id().unwrap_or_else(|| journal::UNKNOWN_BOOT.into()));
            }
            if observe::matching_pid(entry.pid, entry.ticks).is_none() {
                if let Some(agent) = &entry.spec.agent {
                    self.retire_orphans(&agent.sandbox)?;
                }
                // A limits-only launch intent still owns its cgroup and can prove retirement.
                if resources::pending_limits(&entry).is_none() {
                    self.note_legacy_loss(&mut entry)?;
                }
            }
            if before != (entry.network_boot.clone(), entry.network_clean) {
                self.save(entry)?;
            }
        }
        Ok(())
    }

    pub(super) async fn prepare_legacy_stop(&self, name: &str) -> Result<(), RuntimeError> {
        let mut entry = self.entry(name)?;
        if entry.network.is_some() || observe::matching_pid(entry.pid, entry.ticks).is_none() {
            return Ok(());
        }
        entry.network_stop_nonce = self.agent_retirement_challenge(name).await;
        self.save(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertain_descendants_survive_record_removal_until_a_proven_new_boot() {
        let root = tempfile::tempdir().unwrap();
        let boot = uuid::Uuid::new_v4().to_string();
        let marker = root.path().join(journal::NETWORK_HISTORY);
        crate::durable::write_atomic(&marker, boot.as_bytes(), 0o600).unwrap();
        assert!(journal::network_boot_uncertain(root.path(), Some(&boot)).unwrap());
        assert!(journal::network_boot_uncertain(root.path(), None).unwrap());
        assert!(!journal::network_boot_uncertain(root.path(), Some(&uuid::Uuid::new_v4().to_string())).unwrap());
        crate::durable::write_atomic(&marker, journal::UNKNOWN_BOOT.as_bytes(), 0o600).unwrap();
        assert!(journal::network_boot_uncertain(root.path(), Some(&boot)).unwrap());
    }
}
