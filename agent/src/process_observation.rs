// SPDX-License-Identifier: FSL-1.1-ALv2
//! Challenge-bound process observations reuse the sandbox's existing signing key. They let
//! the host authenticate installed isolation without trusting an unsigned local HTTP listener.
use crate::auth::{self, SigningKey};
use serde::{Deserialize, Serialize};

pub const NONCE_HEADER: &str = "x-pryx-process-nonce";
const MAX_NONCE_BYTES: usize = 64;
const CONTEXT: &str = "pryx process observation v1";

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub nonce: String,
    pub sandbox: String,
    pub epoch: u64,
    pub pid: u32,
    pub ipc_scoped: bool,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Proof {
    pub observation: Observation,
    pub signature: String,
}
pub fn nonce_valid(nonce: &str) -> bool {
    !nonce.is_empty()
        && nonce.len() <= MAX_NONCE_BYTES
        && nonce.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
fn bytes(observation: &Observation) -> Vec<u8> {
    serde_json::to_vec(&(CONTEXT, observation)).expect("fixed observation serializes")
}
pub(crate) fn sign(key: &SigningKey, observation: Observation) -> Proof {
    let signature = key.sign_bytes(&bytes(&observation));
    Proof { observation, signature }
}
pub fn verify(proof: &Proof, public_key: &str, nonce: &str, sandbox: &str, epoch: u64, pid: u32) -> bool {
    let observed = &proof.observation;
    nonce_valid(nonce)
        && observed.nonce == nonce
        && observed.sandbox == sandbox
        && observed.epoch == epoch
        && observed.pid == pid
        && observed.ipc_scoped
        && auth::verify_bytes(public_key, &bytes(observed), &proof.signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_observation_cannot_be_forged_replayed_or_reassigned() {
        let (key, _) = SigningKey::generate().unwrap();
        let observation = Observation {
            nonce: "fresh-node-challenge".into(),
            sandbox: "one".into(),
            epoch: 7,
            pid: std::process::id(),
            ipc_scoped: true,
        };
        let proof = sign(&key, observation);
        let verify_here =
            |proof: &Proof, nonce, sandbox, epoch, pid| verify(proof, &key.public_key(), nonce, sandbox, epoch, pid);
        assert!(verify_here(&proof, "fresh-node-challenge", "one", 7, std::process::id()));
        assert!(!verify_here(&proof, "another-challenge", "one", 7, std::process::id()));
        assert!(!verify_here(&proof, "fresh-node-challenge", "other", 7, std::process::id()));
        assert!(!verify_here(&proof, "fresh-node-challenge", "one", 8, std::process::id()));
        assert!(!verify_here(&proof, "fresh-node-challenge", "one", 7, std::process::id() + 1));
        let (other, _) = SigningKey::generate().unwrap();
        assert!(!verify(&proof, &other.public_key(), "fresh-node-challenge", "one", 7, std::process::id()));
        let mut false_proof = sign(&key, Observation { ipc_scoped: false, ..proof.observation.clone() });
        false_proof.observation.ipc_scoped = true;
        assert!(!verify_here(&false_proof, "fresh-node-challenge", "one", 7, std::process::id()));
        let mut forged = proof;
        forged.observation.ipc_scoped = false;
        assert!(!verify_here(&forged, "fresh-node-challenge", "one", 7, std::process::id()));
        forged.observation.ipc_scoped = true;
        forged.signature.clear();
        assert!(!verify_here(&forged, "fresh-node-challenge", "one", 7, std::process::id()));
    }
}
