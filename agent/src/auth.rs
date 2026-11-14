// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! The node signs sandbox capabilities; agents and edges verify with the public key alone.
//! Signing material has no Debug representation and is never part of a protocol response.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::rand::SystemRandom;
use ring::signature::{self, Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};

pub const TOKEN_PREFIX: &str = "pa1";
pub const TOKEN_LIFETIME_SECONDS: i64 = 5 * 60;
pub const TOKEN_CLOCK_SKEW_SECONDS: i64 = 30;
pub const MAX_TOKEN_BYTES: usize = 4096;
pub const FILE_TOKEN_LIFETIME_SECONDS: i64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Sandbox,
    Routes,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Claims {
    pub sandbox: String,
    pub epoch: u64,
    pub issued_at: i64,
    pub expires_at: i64,
    pub scope: Scope,
    pub max_ports: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_exec_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileAccess>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FileAccess {
    pub path: String,
    pub method: FileMethod,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "UPPERCASE")]
pub enum FileMethod {
    Get,
    Put,
}

impl Claims {
    pub fn allows_file(&self, endpoint: &str, method: &str, path: Option<&str>) -> bool {
        self.scope == Scope::File
            && endpoint == "/_pryx/files/content"
            && self.file.as_ref().is_some_and(|file| {
                path == Some(file.path.as_str())
                    && matches!((&file.method, method), (FileMethod::Get, "GET") | (FileMethod::Put, "PUT"))
            })
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("Invalid sandbox token")]
    Invalid,
    #[error("Sandbox token expired")]
    Expired,
    #[error("Sandbox token has another scope")]
    Scope,
}

pub struct SigningKey(Ed25519KeyPair);

impl SigningKey {
    /// The second value goes straight into the container's private bootstrap environment.
    pub fn generate() -> Result<(Self, String), AuthError> {
        let document = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| AuthError::Invalid)?;
        Ok((Self::from_bytes(document.as_ref())?, URL_SAFE_NO_PAD.encode(document.as_ref())))
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, AuthError> {
        Ed25519KeyPair::from_pkcs8(bytes).map(Self).map_err(|_| AuthError::Invalid)
    }

    pub fn from_encoded(encoded: &str) -> Result<Self, AuthError> {
        let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| AuthError::Invalid)?;
        Self::from_bytes(&bytes)
    }

    pub fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0.public_key().as_ref())
    }

    #[cfg(feature = "server")]
    pub(crate) fn sign_bytes(&self, bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(self.0.sign(bytes).as_ref())
    }

    pub fn sign(&self, claims: &Claims) -> Result<String, AuthError> {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).map_err(|_| AuthError::Invalid)?);
        let signed = format!("{TOKEN_PREFIX}.{payload}");
        let token = format!("{signed}.{}", URL_SAFE_NO_PAD.encode(self.0.sign(signed.as_bytes()).as_ref()));
        if token.len() > MAX_TOKEN_BYTES {
            return Err(AuthError::Invalid);
        }
        Ok(token)
    }
}

pub(crate) fn verify_bytes(key: &str, bytes: &[u8], encoded_signature: &str) -> Result<(), AuthError> {
    let public = URL_SAFE_NO_PAD.decode(key).map_err(|_| AuthError::Invalid)?;
    let signature = URL_SAFE_NO_PAD.decode(encoded_signature).map_err(|_| AuthError::Invalid)?;
    signature::UnparsedPublicKey::new(&signature::ED25519, public)
        .verify(bytes, &signature)
        .map_err(|_| AuthError::Invalid)
}

pub fn verify(token: &str, key: &str, sandbox: &str, epoch: u64, now: i64) -> Result<Claims, AuthError> {
    if token.len() > MAX_TOKEN_BYTES {
        return Err(AuthError::Invalid);
    }
    let (signed, encoded_signature) = token.rsplit_once('.').ok_or(AuthError::Invalid)?;
    let (prefix, payload) = signed.split_once('.').ok_or(AuthError::Invalid)?;
    if prefix != TOKEN_PREFIX {
        return Err(AuthError::Invalid);
    }
    verify_bytes(key, signed.as_bytes(), encoded_signature)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| AuthError::Invalid)?;
    let claims: Claims = serde_json::from_slice(&bytes).map_err(|_| AuthError::Invalid)?;
    if claims.sandbox != sandbox || claims.epoch != epoch {
        return Err(AuthError::Scope);
    }
    if claims.expires_at <= now {
        return Err(AuthError::Expired);
    }
    if claims.issued_at > now.saturating_add(TOKEN_CLOCK_SKEW_SECONDS)
        || claims.expires_at <= claims.issued_at
        || claims.expires_at.saturating_sub(claims.issued_at)
            > if claims.scope == Scope::File { FILE_TOKEN_LIFETIME_SECONDS } else { TOKEN_LIFETIME_SECONDS }
    {
        return Err(AuthError::Invalid);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims() -> Claims {
        Claims {
            sandbox: "sandbox-a".into(),
            epoch: 7,
            issued_at: 1000,
            expires_at: 1300,
            scope: Scope::Sandbox,
            max_ports: 3,
            max_exec_seconds: Some(30),
            file: None,
        }
    }

    #[test]
    fn a_token_is_bound_to_key_sandbox_epoch_and_expiry() {
        let (_, encoded) = SigningKey::generate().unwrap();
        let key = SigningKey::from_encoded(&encoded).unwrap();
        let token = key.sign(&claims()).unwrap();
        assert_eq!(verify(&token, &key.public_key(), "sandbox-a", 7, 1100).unwrap(), claims());
        assert_eq!(verify(&token, &key.public_key(), "sandbox-b", 7, 1100), Err(AuthError::Scope));
        assert_eq!(verify(&token, &key.public_key(), "sandbox-a", 8, 1100), Err(AuthError::Scope));
        assert_eq!(verify(&token, &key.public_key(), "sandbox-a", 7, 1300), Err(AuthError::Expired));
        let (other, _) = SigningKey::generate().unwrap();
        assert_eq!(verify(&token, &other.public_key(), "sandbox-a", 7, 1100), Err(AuthError::Invalid));
        assert_ne!(encoded, key.public_key());
    }

    #[test]
    fn signed_claims_cannot_gain_an_unbounded_or_future_lifetime() {
        let (key, _) = SigningKey::generate().unwrap();
        for (issued_at, expires_at) in [(1000, 1301), (1100, 1400), (1000, 1000)] {
            let claims = Claims { issued_at, expires_at, ..claims() };
            let token = key.sign(&claims).unwrap();
            assert!(verify(&token, &key.public_key(), "sandbox-a", 7, 1000).is_err());
        }
    }

    #[test]
    fn malformed_and_oversized_tokens_are_refused() {
        let (key, _) = SigningKey::generate().unwrap();
        for token in ["", "pa1.a", "pa1.a.a", "pa2.a.a", &"a".repeat(MAX_TOKEN_BYTES + 1)] {
            assert_eq!(verify(token, &key.public_key(), "sandbox-a", 7, 1000), Err(AuthError::Invalid));
        }
        assert!(key.sign(&Claims { sandbox: "a".repeat(MAX_TOKEN_BYTES), ..claims() }).is_err());
    }
}
