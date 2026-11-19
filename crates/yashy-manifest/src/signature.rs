//! Package signatures: Ed25519 over a SHA-256 digest.
//!
//! The primitive is a single recorded dependency (`ed25519-compact`, no
//! default features). This module never implements curve arithmetic. A
//! signature names the key that made it and the digest it covers, so a reader
//! can refuse an unknown key or a mismatched body with a distinct error.

use std::collections::BTreeMap;

use ed25519_compact::{KeyPair, PublicKey, Seed, Signature as EdSignature};

use crate::lock::sha256_hex;

/// What a signature covers and who made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub key_id: String,
    pub algorithm: &'static str,
    /// Hex SHA-256 of the signed bytes, without a prefix.
    pub digest: String,
    /// Raw 64-byte Ed25519 signature.
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    UnknownKey(String),
    BadKey(String),
    BadSignature,
    DigestMismatch { expected: String, actual: String },
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::UnknownKey(id) => write!(f, "no trusted key named `{id}`"),
            VerifyError::BadKey(id) => write!(f, "key `{id}` is not a valid Ed25519 public key"),
            VerifyError::BadSignature => f.write_str("the Ed25519 signature is invalid"),
            VerifyError::DigestMismatch { expected, actual } => write!(
                f,
                "the signature covers {expected}, but the content hashes to {actual}"
            ),
        }
    }
}

/// The explicit trust set. An empty keyring trusts nothing.
#[derive(Clone, Debug, Default)]
pub struct Keyring {
    keys: BTreeMap<String, PublicKey>,
}

impl Keyring {
    /// Add a 32-byte raw public key under a caller-chosen id.
    pub fn insert(&mut self, id: impl Into<String>, bytes: &[u8]) -> Result<(), VerifyError> {
        let id = id.into();
        let key = PublicKey::from_slice(bytes).map_err(|_| VerifyError::BadKey(id.clone()))?;
        self.keys.insert(id, key);
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.keys.contains_key(id)
    }
}

#[cfg(test)]
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
        .collect()
}

/// Sign `bytes` with a raw 32-byte seed and name the key.
pub fn sign(bytes: &[u8], key_id: &str, seed: &[u8]) -> Result<Signature, VerifyError> {
    let seed = Seed::from_slice(seed).map_err(|_| VerifyError::BadKey(key_id.to_string()))?;
    let pair = KeyPair::from_seed(seed);
    let signature = pair.sk.sign(bytes, None);
    Ok(Signature {
        key_id: key_id.to_string(),
        algorithm: "Ed25519",
        digest: sha256_hex(bytes),
        bytes: signature.as_ref().to_vec(),
    })
}

/// Verify a signature against the exact bytes it claims to cover.
pub fn verify(bytes: &[u8], signature: &Signature, keys: &Keyring) -> Result<(), VerifyError> {
    let actual = sha256_hex(bytes);
    if actual != signature.digest {
        return Err(VerifyError::DigestMismatch {
            expected: signature.digest.clone(),
            actual,
        });
    }
    let key = keys
        .keys
        .get(&signature.key_id)
        .ok_or_else(|| VerifyError::UnknownKey(signature.key_id.clone()))?;
    let parsed =
        EdSignature::from_slice(&signature.bytes).map_err(|_| VerifyError::BadSignature)?;
    key.verify(bytes, &parsed)
        .map_err(|_| VerifyError::BadSignature)
}

/// The public key of a raw seed, for writing a keyring fixture.
pub fn public_key(seed: &[u8]) -> Result<Vec<u8>, VerifyError> {
    let seed = Seed::from_slice(seed).map_err(|_| VerifyError::BadSignature)?;
    Ok(KeyPair::from_seed(seed).pk.as_ref().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
        26, 27, 28, 29, 30, 31, 32,
    ];

    #[test]
    fn a_valid_signature_verifies() {
        let signature = sign(b"package", "test", &SEED).unwrap();
        let mut keys = Keyring::default();
        keys.insert("test", &public_key(&SEED).unwrap()).unwrap();
        assert!(verify(b"package", &signature, &keys).is_ok());
    }

    #[test]
    fn a_changed_byte_and_an_unknown_key_fail_differently() {
        let signature = sign(b"package", "test", &SEED).unwrap();
        let mut keys = Keyring::default();
        keys.insert("test", &public_key(&SEED).unwrap()).unwrap();
        assert!(matches!(
            verify(b"packagX", &signature, &keys),
            Err(VerifyError::DigestMismatch { .. })
        ));
        assert!(matches!(
            verify(b"package", &signature, &Keyring::default()),
            Err(VerifyError::UnknownKey(_))
        ));
    }

    #[test]
    fn hex_is_decoded_for_stored_signatures() {
        assert_eq!(decode_hex("00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(decode_hex("abc"), None);
    }
}
