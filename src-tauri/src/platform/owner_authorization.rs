use crate::owner_authorization::OWNER_KEY_TAG;
use std::ffi::{c_char, c_void, CString};
use std::ptr;
use std::slice;
use thiserror::Error;

const JARVIS_OWNER_SUCCESS: i32 = 0;
const JARVIS_OWNER_SECURE_ENCLAVE_UNAVAILABLE: i32 = -1;
const JARVIS_OWNER_KEY_NOT_FOUND: i32 = -2;
const JARVIS_OWNER_DUPLICATE_KEY: i32 = -3;
const JARVIS_OWNER_USER_CANCELLED: i32 = -4;
const JARVIS_OWNER_AUTHENTICATION_FAILED: i32 = -5;
const JARVIS_OWNER_INVALID_PUBLIC_KEY: i32 = -6;
const JARVIS_OWNER_INVALID_SIGNATURE: i32 = -7;
const JARVIS_OWNER_SYSTEM: i32 = -8;
const MAX_BRIDGE_DATA_BYTES: usize = 4096;
const MAX_SIGNATURE_BYTES: usize = 256;

type JarvisCfDataRef = *const c_void;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyProof {
    pub public_key_x963: Vec<u8>,
    pub signature_der: Vec<u8>,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum OwnerKeyError {
    #[error("secure enclave is unavailable")]
    SecureEnclaveUnavailable,
    #[error("owner key was not found")]
    KeyNotFound,
    #[error("owner key already exists")]
    DuplicateKey,
    #[error("user cancelled authentication")]
    UserCancelled,
    #[error("owner authentication failed")]
    AuthenticationFailed,
    #[error("owner public key is invalid")]
    InvalidPublicKey,
    #[error("owner signature is invalid")]
    InvalidSignature,
    #[error("owner key platform operation failed")]
    System,
}

pub trait OwnerKeyBackend: Send + Sync {
    fn provision(&self, challenge: &[u8]) -> Result<KeyProof, OwnerKeyError>;
    fn copy_public_key(&self) -> Result<Vec<u8>, OwnerKeyError>;
    fn sign(&self, message: &[u8], prompt: &str) -> Result<KeyProof, OwnerKeyError>;
    fn delete_after_proof(&self, challenge: &[u8], prompt: &str) -> Result<(), OwnerKeyError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawProof {
    public_key_x963: Vec<u8>,
    signature_der: Vec<u8>,
}

trait OwnerKeyBridge: Send + Sync {
    fn provision(&self, tag: &[u8], challenge: &[u8]) -> Result<RawProof, i32>;
    fn copy_public_key(&self, tag: &[u8]) -> Result<Vec<u8>, i32>;
    fn sign(&self, tag: &[u8], message: &[u8], prompt: &str) -> Result<RawProof, i32>;
    fn delete_after_proof(&self, tag: &[u8], challenge: &[u8], prompt: &str) -> Result<(), i32>;
}

pub struct SecurityFrameworkOwnerKeyBackend {
    bridge: Box<dyn OwnerKeyBridge>,
}

impl SecurityFrameworkOwnerKeyBackend {
    pub fn new() -> Self {
        Self {
            bridge: Box::new(NativeOwnerKeyBridge),
        }
    }

    #[cfg(test)]
    fn with_bridge(bridge: impl OwnerKeyBridge + 'static) -> Self {
        Self {
            bridge: Box::new(bridge),
        }
    }
}

impl Default for SecurityFrameworkOwnerKeyBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl OwnerKeyBackend for SecurityFrameworkOwnerKeyBackend {
    fn provision(&self, challenge: &[u8]) -> Result<KeyProof, OwnerKeyError> {
        let proof = self
            .bridge
            .provision(OWNER_KEY_TAG.as_bytes(), challenge)
            .map_err(map_bridge_status)?;
        validate_proof(proof)
    }

    fn copy_public_key(&self) -> Result<Vec<u8>, OwnerKeyError> {
        let public_key = self
            .bridge
            .copy_public_key(OWNER_KEY_TAG.as_bytes())
            .map_err(map_bridge_status)?;
        validate_public_key(&public_key)?;
        Ok(public_key)
    }

    fn sign(&self, message: &[u8], prompt: &str) -> Result<KeyProof, OwnerKeyError> {
        let proof = self
            .bridge
            .sign(OWNER_KEY_TAG.as_bytes(), message, prompt)
            .map_err(map_bridge_status)?;
        validate_proof(proof)
    }

    fn delete_after_proof(&self, challenge: &[u8], prompt: &str) -> Result<(), OwnerKeyError> {
        self.bridge
            .delete_after_proof(OWNER_KEY_TAG.as_bytes(), challenge, prompt)
            .map_err(map_bridge_status)
    }
}

struct NativeOwnerKeyBridge;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn jarvis_owner_key_provision(
        tag: *const u8,
        tag_len: usize,
        challenge: *const u8,
        challenge_len: usize,
        public_key_out: *mut JarvisCfDataRef,
        signature_out: *mut JarvisCfDataRef,
    ) -> i32;
    fn jarvis_owner_key_copy_public(
        tag: *const u8,
        tag_len: usize,
        public_key_out: *mut JarvisCfDataRef,
    ) -> i32;
    fn jarvis_owner_key_sign(
        tag: *const u8,
        tag_len: usize,
        message: *const u8,
        message_len: usize,
        prompt_utf8: *const c_char,
        public_key_out: *mut JarvisCfDataRef,
        signature_out: *mut JarvisCfDataRef,
    ) -> i32;
    fn jarvis_owner_key_delete_after_proof(
        tag: *const u8,
        tag_len: usize,
        challenge: *const u8,
        challenge_len: usize,
        prompt_utf8: *const c_char,
    ) -> i32;
    fn jarvis_owner_cf_release(value: *const c_void);
    fn CFDataGetLength(data: JarvisCfDataRef) -> isize;
    fn CFDataGetBytePtr(data: JarvisCfDataRef) -> *const u8;
}

#[cfg(target_os = "macos")]
impl OwnerKeyBridge for NativeOwnerKeyBridge {
    fn provision(&self, tag: &[u8], challenge: &[u8]) -> Result<RawProof, i32> {
        let mut public_key = ptr::null();
        let mut signature = ptr::null();
        // SAFETY: slices remain valid for the call and output pointers reference local storage.
        let status = unsafe {
            jarvis_owner_key_provision(
                tag.as_ptr(),
                tag.len(),
                challenge.as_ptr(),
                challenge.len(),
                &mut public_key,
                &mut signature,
            )
        };
        bridge_proof(status, public_key, signature)
    }

    fn copy_public_key(&self, tag: &[u8]) -> Result<Vec<u8>, i32> {
        let mut public_key = ptr::null();
        // SAFETY: tag remains valid for the call and output points to local storage.
        let status =
            unsafe { jarvis_owner_key_copy_public(tag.as_ptr(), tag.len(), &mut public_key) };
        bridge_data(status, public_key)
    }

    fn sign(&self, tag: &[u8], message: &[u8], prompt: &str) -> Result<RawProof, i32> {
        let prompt = CString::new(prompt).map_err(|_| JARVIS_OWNER_SYSTEM)?;
        let mut public_key = ptr::null();
        let mut signature = ptr::null();
        // SAFETY: all inputs remain valid for the call and outputs reference local storage.
        let status = unsafe {
            jarvis_owner_key_sign(
                tag.as_ptr(),
                tag.len(),
                message.as_ptr(),
                message.len(),
                prompt.as_ptr(),
                &mut public_key,
                &mut signature,
            )
        };
        bridge_proof(status, public_key, signature)
    }

    fn delete_after_proof(&self, tag: &[u8], challenge: &[u8], prompt: &str) -> Result<(), i32> {
        let prompt = CString::new(prompt).map_err(|_| JARVIS_OWNER_SYSTEM)?;
        // SAFETY: all inputs remain valid for the duration of the bridge call.
        let status = unsafe {
            jarvis_owner_key_delete_after_proof(
                tag.as_ptr(),
                tag.len(),
                challenge.as_ptr(),
                challenge.len(),
                prompt.as_ptr(),
            )
        };
        if status == JARVIS_OWNER_SUCCESS {
            Ok(())
        } else {
            Err(status)
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl OwnerKeyBridge for NativeOwnerKeyBridge {
    fn provision(&self, _tag: &[u8], _challenge: &[u8]) -> Result<RawProof, i32> {
        Err(JARVIS_OWNER_SECURE_ENCLAVE_UNAVAILABLE)
    }

    fn copy_public_key(&self, _tag: &[u8]) -> Result<Vec<u8>, i32> {
        Err(JARVIS_OWNER_SECURE_ENCLAVE_UNAVAILABLE)
    }

    fn sign(&self, _tag: &[u8], _message: &[u8], _prompt: &str) -> Result<RawProof, i32> {
        Err(JARVIS_OWNER_SECURE_ENCLAVE_UNAVAILABLE)
    }

    fn delete_after_proof(&self, _tag: &[u8], _challenge: &[u8], _prompt: &str) -> Result<(), i32> {
        Err(JARVIS_OWNER_SECURE_ENCLAVE_UNAVAILABLE)
    }
}

#[cfg(target_os = "macos")]
fn bridge_proof(
    status: i32,
    public_key: JarvisCfDataRef,
    signature: JarvisCfDataRef,
) -> Result<RawProof, i32> {
    if status != JARVIS_OWNER_SUCCESS {
        release_cf_data(public_key);
        release_cf_data(signature);
        return Err(status);
    }
    let public_key_x963 = match copy_cf_data(public_key) {
        Ok(value) => value,
        Err(error) => {
            release_cf_data(signature);
            return Err(error);
        }
    };
    let signature_der = copy_cf_data(signature)?;
    Ok(RawProof {
        public_key_x963,
        signature_der,
    })
}

#[cfg(target_os = "macos")]
fn bridge_data(status: i32, value: JarvisCfDataRef) -> Result<Vec<u8>, i32> {
    if status != JARVIS_OWNER_SUCCESS {
        release_cf_data(value);
        return Err(status);
    }
    copy_cf_data(value)
}

#[cfg(target_os = "macos")]
fn copy_cf_data(value: JarvisCfDataRef) -> Result<Vec<u8>, i32> {
    let guard = CfDataGuard(value);
    if guard.0.is_null() {
        return Err(JARVIS_OWNER_SYSTEM);
    }
    // SAFETY: guard holds a live CFData returned by the C bridge.
    let length = unsafe { CFDataGetLength(guard.0) };
    let length = usize::try_from(length).map_err(|_| JARVIS_OWNER_SYSTEM)?;
    if length > MAX_BRIDGE_DATA_BYTES {
        return Err(JARVIS_OWNER_SYSTEM);
    }
    if length == 0 {
        return Ok(Vec::new());
    }
    // SAFETY: guard holds a live CFData; the returned pointer remains valid while guard lives.
    let bytes = unsafe { CFDataGetBytePtr(guard.0) };
    if bytes.is_null() {
        return Err(JARVIS_OWNER_SYSTEM);
    }
    // SAFETY: CFData exposes exactly length bytes at a non-null pointer while guard lives.
    Ok(unsafe { slice::from_raw_parts(bytes, length) }.to_vec())
}

#[cfg(target_os = "macos")]
struct CfDataGuard(JarvisCfDataRef);

#[cfg(target_os = "macos")]
impl Drop for CfDataGuard {
    fn drop(&mut self) {
        release_cf_data(self.0);
    }
}

#[cfg(target_os = "macos")]
fn release_cf_data(value: JarvisCfDataRef) {
    if !value.is_null() {
        // SAFETY: values returned by the bridge are released only through its release function.
        unsafe { jarvis_owner_cf_release(value) };
    }
}

fn validate_proof(proof: RawProof) -> Result<KeyProof, OwnerKeyError> {
    validate_public_key(&proof.public_key_x963)?;
    if proof.signature_der.len() < 2
        || proof.signature_der.len() > MAX_SIGNATURE_BYTES
        || proof.signature_der[0] != 0x30
    {
        return Err(OwnerKeyError::InvalidSignature);
    }
    Ok(KeyProof {
        public_key_x963: proof.public_key_x963,
        signature_der: proof.signature_der,
    })
}

fn validate_public_key(public_key: &[u8]) -> Result<(), OwnerKeyError> {
    if public_key.len() != 65 || public_key[0] != 0x04 {
        return Err(OwnerKeyError::InvalidPublicKey);
    }
    Ok(())
}

fn map_bridge_status(status: i32) -> OwnerKeyError {
    match status {
        JARVIS_OWNER_SECURE_ENCLAVE_UNAVAILABLE => OwnerKeyError::SecureEnclaveUnavailable,
        JARVIS_OWNER_KEY_NOT_FOUND => OwnerKeyError::KeyNotFound,
        JARVIS_OWNER_DUPLICATE_KEY => OwnerKeyError::DuplicateKey,
        JARVIS_OWNER_USER_CANCELLED => OwnerKeyError::UserCancelled,
        JARVIS_OWNER_AUTHENTICATION_FAILED => OwnerKeyError::AuthenticationFailed,
        JARVIS_OWNER_INVALID_PUBLIC_KEY => OwnerKeyError::InvalidPublicKey,
        JARVIS_OWNER_INVALID_SIGNATURE => OwnerKeyError::InvalidSignature,
        JARVIS_OWNER_SYSTEM | JARVIS_OWNER_SUCCESS => OwnerKeyError::System,
        _ => OwnerKeyError::System,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner_authorization::OWNER_KEY_TAG;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Provision {
            tag: Vec<u8>,
            challenge: Vec<u8>,
        },
        CopyPublic {
            tag: Vec<u8>,
        },
        Sign {
            tag: Vec<u8>,
            message: Vec<u8>,
            prompt: String,
        },
        DeleteProofSign {
            tag: Vec<u8>,
            challenge: Vec<u8>,
            prompt: String,
        },
        DeleteExact {
            tag: Vec<u8>,
        },
    }

    struct FakeBridge {
        calls: Arc<Mutex<Vec<Call>>>,
        provision: Result<RawProof, i32>,
        public_key: Result<Vec<u8>, i32>,
        sign: Result<RawProof, i32>,
        delete_after_proof: Result<(), i32>,
    }

    impl OwnerKeyBridge for FakeBridge {
        fn provision(&self, tag: &[u8], challenge: &[u8]) -> Result<RawProof, i32> {
            self.calls.lock().unwrap().push(Call::Provision {
                tag: tag.to_vec(),
                challenge: challenge.to_vec(),
            });
            self.provision.clone()
        }

        fn copy_public_key(&self, tag: &[u8]) -> Result<Vec<u8>, i32> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::CopyPublic { tag: tag.to_vec() });
            self.public_key.clone()
        }

        fn sign(&self, tag: &[u8], message: &[u8], prompt: &str) -> Result<RawProof, i32> {
            self.calls.lock().unwrap().push(Call::Sign {
                tag: tag.to_vec(),
                message: message.to_vec(),
                prompt: prompt.to_owned(),
            });
            self.sign.clone()
        }

        fn delete_after_proof(
            &self,
            tag: &[u8],
            challenge: &[u8],
            prompt: &str,
        ) -> Result<(), i32> {
            self.calls.lock().unwrap().push(Call::DeleteProofSign {
                tag: tag.to_vec(),
                challenge: challenge.to_vec(),
                prompt: prompt.to_owned(),
            });
            self.delete_after_proof?;
            self.calls
                .lock()
                .unwrap()
                .push(Call::DeleteExact { tag: tag.to_vec() });
            Ok(())
        }
    }

    #[test]
    fn secure_enclave_unavailable_fails_closed_without_retry() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = backend(
            calls.clone(),
            Err(JARVIS_OWNER_SECURE_ENCLAVE_UNAVAILABLE),
            valid_proof(),
        );

        assert_eq!(
            backend.provision(b"bootstrap"),
            Err(OwnerKeyError::SecureEnclaveUnavailable)
        );
        assert_eq!(
            *calls.lock().unwrap(),
            [Call::Provision {
                tag: OWNER_KEY_TAG.as_bytes().to_vec(),
                challenge: b"bootstrap".to_vec(),
            }]
        );
    }

    #[test]
    fn provision_requires_uncompressed_p256_public_key_and_der_signature() {
        for invalid in [
            RawProof {
                public_key_x963: vec![0x04; 64],
                signature_der: vec![0x30, 0x01],
            },
            RawProof {
                public_key_x963: vec![0x03; 65],
                signature_der: vec![0x30, 0x01],
            },
        ] {
            let backend = backend(Arc::new(Mutex::new(Vec::new())), Ok(invalid), valid_proof());
            assert_eq!(
                backend.provision(b"bootstrap"),
                Err(OwnerKeyError::InvalidPublicKey)
            );
        }

        let backend = backend(
            Arc::new(Mutex::new(Vec::new())),
            Ok(RawProof {
                public_key_x963: valid_public_key(),
                signature_der: Vec::new(),
            }),
            valid_proof(),
        );
        assert_eq!(
            backend.provision(b"bootstrap"),
            Err(OwnerKeyError::InvalidSignature)
        );
    }

    #[test]
    fn sign_returns_the_current_public_key_with_each_signature() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let expected = valid_proof();
        let backend = backend(calls.clone(), Ok(expected.clone()), expected.clone());

        assert_eq!(
            backend.sign(b"message", "Confirm Stage 1 authorization"),
            Ok(KeyProof {
                public_key_x963: expected.public_key_x963,
                signature_der: expected.signature_der,
            })
        );
        assert_eq!(
            *calls.lock().unwrap(),
            [Call::Sign {
                tag: OWNER_KEY_TAG.as_bytes().to_vec(),
                message: b"message".to_vec(),
                prompt: "Confirm Stage 1 authorization".to_owned(),
            }]
        );
    }

    #[test]
    fn cancellation_and_authentication_failure_remain_distinct() {
        for (status, expected) in [
            (JARVIS_OWNER_USER_CANCELLED, OwnerKeyError::UserCancelled),
            (
                JARVIS_OWNER_AUTHENTICATION_FAILED,
                OwnerKeyError::AuthenticationFailed,
            ),
        ] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let mut fake = fake(calls, Ok(valid_proof()), valid_proof());
            fake.sign = Err(status);
            let backend = SecurityFrameworkOwnerKeyBackend::with_bridge(fake);
            assert_eq!(backend.sign(b"message", "prompt"), Err(expected));
        }
    }

    #[test]
    fn delete_proves_once_before_exact_delete_and_skips_delete_on_proof_failure() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = backend(calls.clone(), Ok(valid_proof()), valid_proof());

        backend
            .delete_after_proof(b"delete-challenge", "Confirm deletion")
            .unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            [
                Call::DeleteProofSign {
                    tag: OWNER_KEY_TAG.as_bytes().to_vec(),
                    challenge: b"delete-challenge".to_vec(),
                    prompt: "Confirm deletion".to_owned(),
                },
                Call::DeleteExact {
                    tag: OWNER_KEY_TAG.as_bytes().to_vec(),
                },
            ]
        );

        let failed_calls = Arc::new(Mutex::new(Vec::new()));
        let mut failed = fake(failed_calls.clone(), Ok(valid_proof()), valid_proof());
        failed.delete_after_proof = Err(JARVIS_OWNER_AUTHENTICATION_FAILED);
        let failed_backend = SecurityFrameworkOwnerKeyBackend::with_bridge(failed);
        assert_eq!(
            failed_backend.delete_after_proof(b"delete-challenge", "Confirm deletion"),
            Err(OwnerKeyError::AuthenticationFailed)
        );
        assert_eq!(
            *failed_calls.lock().unwrap(),
            [Call::DeleteProofSign {
                tag: OWNER_KEY_TAG.as_bytes().to_vec(),
                challenge: b"delete-challenge".to_vec(),
                prompt: "Confirm deletion".to_owned(),
            }]
        );
    }

    #[test]
    fn copy_public_key_uses_the_exact_tag_once() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = backend(calls.clone(), Ok(valid_proof()), valid_proof());

        assert_eq!(backend.copy_public_key(), Ok(valid_public_key()));
        assert_eq!(
            *calls.lock().unwrap(),
            [Call::CopyPublic {
                tag: OWNER_KEY_TAG.as_bytes().to_vec(),
            }]
        );
    }

    fn backend(
        calls: Arc<Mutex<Vec<Call>>>,
        provision: Result<RawProof, i32>,
        sign: RawProof,
    ) -> SecurityFrameworkOwnerKeyBackend {
        SecurityFrameworkOwnerKeyBackend::with_bridge(fake(calls, provision, sign))
    }

    fn fake(
        calls: Arc<Mutex<Vec<Call>>>,
        provision: Result<RawProof, i32>,
        sign: RawProof,
    ) -> FakeBridge {
        FakeBridge {
            calls,
            provision,
            public_key: Ok(valid_public_key()),
            sign: Ok(sign),
            delete_after_proof: Ok(()),
        }
    }

    fn valid_proof() -> RawProof {
        RawProof {
            public_key_x963: valid_public_key(),
            signature_der: vec![0x30, 0x01],
        }
    }

    fn valid_public_key() -> Vec<u8> {
        let mut key = vec![0x11; 65];
        key[0] = 0x04;
        key
    }
}
