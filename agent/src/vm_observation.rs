// SPDX-License-Identifier: FSL-1.1-ALv2
//! Guest init signs boot-scoped disk-shutdown observations with the existing agent key. Image
//! output cannot forge a clean shutdown or replay another boot's exit through the serial log.
use crate::{
    auth::{self, SigningKey},
    process_observation::nonce_valid,
};
use serde::{Deserialize, Serialize};

pub const PREFIX: &str = "PRYX_VM_OBSERVATION:";
const CONTEXT: &str = "pryx VM observation v1";

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Observation {
    pub nonce: String,
    pub sandbox: String,
    pub epoch: u64,
    pub kernel: String,
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub disk_synced: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub observation: Observation,
    pub signature: String,
}

fn bytes(observation: &Observation) -> Vec<u8> {
    serde_json::to_vec(&(CONTEXT, observation)).expect("fixed VM observation serializes")
}

pub(crate) fn sign(key: &SigningKey, observation: Observation) -> Proof {
    let signature = key.sign_bytes(&bytes(&observation));
    Proof { observation, signature }
}

pub fn verify(proof: &Proof, public_key: &str, nonce: &str, sandbox: &str, epoch: u64) -> bool {
    let observation = &proof.observation;
    nonce_valid(nonce)
        && observation.nonce == nonce
        && observation.sandbox == sandbox
        && observation.epoch == epoch
        && observation.disk_synced
        && auth::verify_bytes(public_key, &bytes(observation), &proof.signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn shutdown_proof_is_bound_to_boot_scope_and_whole_disks() {
        let (key, _) = SigningKey::generate().unwrap();
        let random = ring::rand::SystemRandom::new();
        let nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(ring::rand::generate::<[u8; 32]>(&random).unwrap().expose());
        let other_nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(ring::rand::generate::<[u8; 32]>(&random).unwrap().expose());
        assert_ne!(nonce, other_nonce);
        let proof = sign(
            &key,
            Observation {
                nonce: nonce.clone(),
                sandbox: "one".into(),
                epoch: 9,
                kernel: "guest-kernel".into(),
                code: Some(0),
                signal: None,
                disk_synced: true,
            },
        );
        assert!(verify(&proof, &key.public_key(), &nonce, "one", 9));
        assert!(!verify(&proof, &key.public_key(), &other_nonce, "one", 9));
        assert!(!verify(&proof, &key.public_key(), &nonce, "other", 9));
        assert!(!verify(&proof, &key.public_key(), &nonce, "one", 10));
        let mut tampered = proof.clone();
        tampered.observation.code = Some(1);
        assert!(!verify(&tampered, &key.public_key(), &nonce, "one", 9));
        let incomplete = sign(&key, Observation { disk_synced: false, ..proof.observation });
        assert!(!verify(&incomplete, &key.public_key(), &nonce, "one", 9));
    }
}
