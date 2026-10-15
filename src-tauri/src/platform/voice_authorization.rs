use crate::voice_authorization::{
    VoiceAuthorizationError, VoiceKeyProof, VoiceKeySession, VoiceOwnerKey,
    VOICE_KEY_APPLICATION_TAG,
};
use std::ffi::{c_char, c_void, CString};
use std::ptr;
use std::slice;
use std::sync::{Arc, Mutex};

const SUCCESS: i32 = 0;
const UNAVAILABLE: i32 = -1;
const KEY_NOT_FOUND: i32 = -2;
const CANCELLED: i32 = -3;
const AUTHENTICATION_FAILED: i32 = -4;
const INVALID_INPUT: i32 = -5;
const SYSTEM: i32 = -6;
const BAD_SIGNATURE: i32 = -7;
const MAX_MESSAGE_BYTES: usize = 65_536;
const MAX_SIGNATURE_BYTES: usize = 256;
const X963_PUBLIC_KEY_BYTES: usize = 65;

const MAX_AUTHORIZATION_STATE_BYTES: usize = 8_192;

type CfDataRef = *const c_void;

pub(crate) struct NativeVoiceKeySession {
    context: Mutex<*mut c_void>,
    public_key_x963: Vec<u8>,
}

// The mutex serializes all uses of the retained LAContext; native callbacks never retain
// pointers into Rust storage and invalidation is performed before releasing the context.
unsafe impl Send for NativeVoiceKeySession {}
unsafe impl Sync for NativeVoiceKeySession {}

impl VoiceKeySession for NativeVoiceKeySession {
    fn sign(&self, message: &[u8]) -> Result<VoiceKeyProof, VoiceAuthorizationError> {
        validate_message(message)?;
        let context = self
            .context
            .lock()
            .map_err(|_| VoiceAuthorizationError::OwnerAuthentication)?;
        if context.is_null() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        #[cfg(target_os = "macos")]
        {
            let mut public_key = ptr::null();
            let mut signature = ptr::null();
            let status = unsafe {
                saturday_voice_key_sign_authorized(
                    VOICE_KEY_APPLICATION_TAG.as_ptr(),
                    VOICE_KEY_APPLICATION_TAG.len(),
                    message.as_ptr(),
                    message.len(),
                    *context,
                    &mut public_key,
                    &mut signature,
                )
            };
            let proof = proof_from_bridge(status, public_key, signature)?;
            if proof.public_key_x963 != self.public_key_x963 {
                return Err(VoiceAuthorizationError::KeyUnavailable);
            }
            Ok(proof)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = message;
            Err(VoiceAuthorizationError::KeyUnavailable)
        }
    }

    fn invalidate(&self) {
        if let Ok(mut context) = self.context.lock() {
            if !context.is_null() {
                #[cfg(target_os = "macos")]
                unsafe {
                    saturday_voice_key_context_invalidate_release(*context)
                };
                *context = ptr::null_mut();
            }
        }
    }

    fn is_valid(&self) -> bool {
        self.context
            .lock()
            .map(|context| !context.is_null())
            .unwrap_or(false)
    }
}

impl Drop for NativeVoiceKeySession {
    fn drop(&mut self) {
        self.invalidate();
    }
}

/// The only production voice signing backend. It uses a dedicated Secure Enclave key
/// application tag; no file, environment, software, or Stage 1 key fallback exists.
pub struct SecurityFrameworkVoiceOwnerKey;

impl SecurityFrameworkVoiceOwnerKey {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeIdentityDigest {
    pub team_id_sha256: String,
    pub designated_requirement_sha256: String,
    pub bundle_identifier: String,
}

pub fn current_code_identity() -> Result<CodeIdentityDigest, VoiceAuthorizationError> {
    copy_code_identity(None)
}

pub fn code_identity_at(
    path: &std::path::Path,
) -> Result<CodeIdentityDigest, VoiceAuthorizationError> {
    copy_code_identity(Some(path))
}

fn copy_code_identity(
    path: Option<&std::path::Path>,
) -> Result<CodeIdentityDigest, VoiceAuthorizationError> {
    #[cfg(target_os = "macos")]
    {
        let path = path
            .map(|path| {
                let value = path
                    .to_str()
                    .ok_or(VoiceAuthorizationError::ProtocolFailure)?;
                if value.is_empty() || value.len() > 4096 || value.contains('\0') {
                    return Err(VoiceAuthorizationError::ProtocolFailure);
                }
                CString::new(value).map_err(|_| VoiceAuthorizationError::ProtocolFailure)
            })
            .transpose()?;
        let mut team = ptr::null();
        let mut requirement = ptr::null();
        let mut bundle_identifier = ptr::null();
        // SAFETY: the optional NUL-terminated path remains valid for this synchronous call;
        // native outputs are retained CFData copied and released by data_from_bridge.
        let status = unsafe {
            saturday_voice_copy_code_identity(
                path.as_ref().map_or(ptr::null(), |path| path.as_ptr()),
                &mut team,
                &mut requirement,
                &mut bundle_identifier,
            )
        };
        let team_result = data_from_bridge(status, team);
        let requirement_result = data_from_bridge(status, requirement);
        let bundle_result = data_from_bridge(status, bundle_identifier);
        let team = team_result?;
        let requirement = requirement_result?;
        let bundle_identifier = bundle_result?;
        let bundle_identifier = std::str::from_utf8(&bundle_identifier)
            .map_err(|_| VoiceAuthorizationError::ProtocolFailure)?;
        if team.is_empty()
            || std::str::from_utf8(&team).is_err()
            || bundle_identifier.is_empty()
            || bundle_identifier.len() > 256
            || bundle_identifier.contains('\0')
        {
            return Err(VoiceAuthorizationError::ProtocolFailure);
        }
        Ok(CodeIdentityDigest {
            team_id_sha256: crate::voice_authorization::sha256_hex(&team),
            designated_requirement_sha256: crate::voice_authorization::sha256_hex(&requirement),
            bundle_identifier: bundle_identifier.to_owned(),
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err(VoiceAuthorizationError::KeyUnavailable)
    }
}

impl Default for SecurityFrameworkVoiceOwnerKey {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceOwnerKey for SecurityFrameworkVoiceOwnerKey {
    fn provision(
        &self,
        challenge: &[u8],
        prompt: &str,
    ) -> Result<VoiceKeyProof, VoiceAuthorizationError> {
        validate_message(challenge)?;
        let prompt = prompt_string(prompt)?;
        #[cfg(target_os = "macos")]
        {
            let mut public_key = ptr::null();
            let mut signature = ptr::null();
            // SAFETY: all input slices and the prompt remain valid until the C function
            // returns; output pointers refer to initialized local slots and returned CFData
            // is copied and released below.
            let status = unsafe {
                saturday_voice_key_provision(
                    VOICE_KEY_APPLICATION_TAG.as_ptr(),
                    VOICE_KEY_APPLICATION_TAG.len(),
                    challenge.as_ptr(),
                    challenge.len(),
                    prompt.as_ptr(),
                    &mut public_key,
                    &mut signature,
                )
            };
            proof_from_bridge(status, public_key, signature)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = prompt;
            Err(VoiceAuthorizationError::KeyUnavailable)
        }
    }

    fn public_key(&self, _prompt: &str) -> Result<Vec<u8>, VoiceAuthorizationError> {
        #[cfg(target_os = "macos")]
        {
            let mut public_key = ptr::null();
            // SAFETY: the fixed application tag is valid for the call and output points
            // to local storage. The native bridge returns retained CFData.
            let status = unsafe {
                saturday_voice_key_copy_public(
                    VOICE_KEY_APPLICATION_TAG.as_ptr(),
                    VOICE_KEY_APPLICATION_TAG.len(),
                    &mut public_key,
                )
            };
            let bytes = data_from_bridge(status, public_key)?;
            validate_public_key(&bytes)?;
            Ok(bytes)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(VoiceAuthorizationError::KeyUnavailable)
        }
    }

    fn sign(&self, message: &[u8], prompt: &str) -> Result<VoiceKeyProof, VoiceAuthorizationError> {
        validate_message(message)?;
        let prompt = prompt_string(prompt)?;
        #[cfg(target_os = "macos")]
        {
            let mut public_key = ptr::null();
            let mut signature = ptr::null();
            // SAFETY: all input buffers remain valid until return; native outputs are owned
            // retained CFData objects copied by proof_from_bridge.
            let status = unsafe {
                saturday_voice_key_sign(
                    VOICE_KEY_APPLICATION_TAG.as_ptr(),
                    VOICE_KEY_APPLICATION_TAG.len(),
                    message.as_ptr(),
                    message.len(),
                    prompt.as_ptr(),
                    &mut public_key,
                    &mut signature,
                )
            };
            proof_from_bridge(status, public_key, signature)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = prompt;
            Err(VoiceAuthorizationError::KeyUnavailable)
        }
    }

    fn preauthorize(
        &self,
        prompt: &str,
    ) -> Result<Arc<dyn VoiceKeySession>, VoiceAuthorizationError> {
        let prompt = prompt_string(prompt)?;
        #[cfg(target_os = "macos")]
        {
            let mut context = ptr::null_mut();
            let status = unsafe {
                saturday_voice_key_authorize_session(
                    VOICE_KEY_APPLICATION_TAG.as_ptr(),
                    VOICE_KEY_APPLICATION_TAG.len(),
                    prompt.as_ptr(),
                    &mut context,
                )
            };
            if status != SUCCESS || context.is_null() {
                if !context.is_null() {
                    unsafe { saturday_voice_key_context_invalidate_release(context) };
                }
                return Err(map_status(status));
            }
            let public_key_x963 = match self.public_key("") {
                Ok(value) => value,
                Err(error) => {
                    unsafe { saturday_voice_key_context_invalidate_release(context) };
                    return Err(error);
                }
            };
            Ok(Arc::new(NativeVoiceKeySession {
                context: Mutex::new(context),
                public_key_x963,
            }))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = prompt;
            Err(VoiceAuthorizationError::KeyUnavailable)
        }
    }

    fn verify(
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<bool, VoiceAuthorizationError> {
        validate_public_key(public_key)?;
        validate_message(message)?;
        if signature.len() < 8 || signature.len() > MAX_SIGNATURE_BYTES || signature[0] != 0x30 {
            return Err(VoiceAuthorizationError::ProtocolFailure);
        }
        #[cfg(target_os = "macos")]
        {
            let mut valid = 0;
            // SAFETY: all input slices remain valid for the synchronous native verification
            // and the output pointer refers to local initialized storage.
            let status = unsafe {
                saturday_voice_key_verify(
                    public_key.as_ptr(),
                    public_key.len(),
                    message.as_ptr(),
                    message.len(),
                    signature.as_ptr(),
                    signature.len(),
                    &mut valid,
                )
            };
            if status == SUCCESS {
                Ok(valid == 1)
            } else {
                Err(map_status(status))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(VoiceAuthorizationError::KeyUnavailable)
        }
    }
}

fn validate_message(message: &[u8]) -> Result<(), VoiceAuthorizationError> {
    if message.is_empty() || message.len() > MAX_MESSAGE_BYTES {
        return Err(VoiceAuthorizationError::ProtocolFailure);
    }
    Ok(())
}

fn prompt_string(prompt: &str) -> Result<CString, VoiceAuthorizationError> {
    if prompt.is_empty() || prompt.len() > 256 || prompt.contains('\0') {
        return Err(VoiceAuthorizationError::ProtocolFailure);
    }
    CString::new(prompt).map_err(|_| VoiceAuthorizationError::ProtocolFailure)
}

fn validate_public_key(value: &[u8]) -> Result<(), VoiceAuthorizationError> {
    if value.len() != X963_PUBLIC_KEY_BYTES || value.first() != Some(&0x04) {
        return Err(VoiceAuthorizationError::ProtocolFailure);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn saturday_voice_key_provision(
        tag: *const u8,
        tag_len: usize,
        challenge: *const u8,
        challenge_len: usize,
        prompt: *const c_char,
        public_out: *mut CfDataRef,
        signature_out: *mut CfDataRef,
    ) -> i32;
    fn saturday_voice_key_copy_public(
        tag: *const u8,
        tag_len: usize,
        public_out: *mut CfDataRef,
    ) -> i32;
    fn saturday_voice_key_sign(
        tag: *const u8,
        tag_len: usize,
        message: *const u8,
        message_len: usize,
        prompt: *const c_char,
        public_out: *mut CfDataRef,
        signature_out: *mut CfDataRef,
    ) -> i32;
    fn saturday_voice_key_verify(
        public_key: *const u8,
        public_len: usize,
        message: *const u8,
        message_len: usize,
        signature: *const u8,
        signature_len: usize,
        valid_out: *mut i32,
    ) -> i32;
    fn saturday_voice_key_authorize_session(
        tag: *const u8,
        tag_len: usize,
        prompt: *const c_char,
        context_out: *mut *mut c_void,
    ) -> i32;
    fn saturday_voice_key_sign_authorized(
        tag: *const u8,
        tag_len: usize,
        message: *const u8,
        message_len: usize,
        context: *mut c_void,
        public_out: *mut CfDataRef,
        signature_out: *mut CfDataRef,
    ) -> i32;
    fn saturday_voice_key_context_invalidate_release(context: *mut c_void);
    fn saturday_voice_copy_code_identity(
        path: *const c_char,
        team_out: *mut CfDataRef,
        requirement_out: *mut CfDataRef,
        bundle_identifier_out: *mut CfDataRef,
    ) -> i32;
    fn saturday_voice_authorization_state_copy(
        output: *mut u8,
        capacity: usize,
        length_out: *mut usize,
        serial_out: *mut u64,
    ) -> i32;
    fn saturday_voice_authorization_state_store(
        serial: u64,
        json_bytes: *const u8,
        json_length: usize,
    ) -> i32;
    fn saturday_voice_keychain_app_scope_self_test() -> i32;
    fn saturday_voice_cf_release(value: CfDataRef);
    fn CFDataGetLength(data: CfDataRef) -> isize;

    fn CFDataGetBytePtr(data: CfDataRef) -> *const u8;
}
pub(crate) fn verify_app_scoped_keychain() -> Result<(), VoiceAuthorizationError> {
    #[cfg(target_os = "macos")]
    {
        let status = unsafe { saturday_voice_keychain_app_scope_self_test() };
        if status == SUCCESS {
            Ok(())
        } else {
            Err(map_status(status))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(VoiceAuthorizationError::KeyUnavailable)
    }
}

pub(crate) fn read_protected_authorization_state(
) -> Result<Option<Vec<u8>>, VoiceAuthorizationError> {
    #[cfg(target_os = "macos")]
    {
        let mut bytes = vec![0; MAX_AUTHORIZATION_STATE_BYTES];
        let mut length = 0usize;
        let mut serial = 0u64;
        let status = unsafe {
            saturday_voice_authorization_state_copy(
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut length,
                &mut serial,
            )
        };
        if status == KEY_NOT_FOUND {
            return Ok(None);
        }
        if status != SUCCESS {
            return Err(map_status(status));
        }
        if length == 0 || length > bytes.len() {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        bytes.truncate(length);
        validate_protected_state(&bytes, serial)?;
        Ok(Some(bytes))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(VoiceAuthorizationError::KeyUnavailable)
    }
}

pub(crate) fn store_protected_authorization_state(
    bytes: &[u8],
    serial: u64,
) -> Result<(), VoiceAuthorizationError> {
    if bytes.is_empty() || bytes.len() > MAX_AUTHORIZATION_STATE_BYTES || serial == 0 {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    validate_protected_state(bytes, serial)?;
    #[cfg(target_os = "macos")]
    {
        let status = unsafe {
            saturday_voice_authorization_state_store(serial, bytes.as_ptr(), bytes.len())
        };
        if status == SUCCESS {
            Ok(())
        } else {
            Err(map_status(status))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(VoiceAuthorizationError::KeyUnavailable)
    }
}

fn validate_protected_state(bytes: &[u8], serial: u64) -> Result<(), VoiceAuthorizationError> {
    let state: saturday_voice_parent_auth::SignedAuthorizationState =
        serde_json::from_slice(bytes).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
    let canonical_value =
        serde_json::to_value(&state).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
    let canonical = serde_json::to_vec(&canonical_value)
        .map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
    if canonical != bytes
        || state.payload.validate().is_err()
        || state.payload.serial != serial
        || state.signature_der.len() < 8
        || state.signature_der.len() > MAX_SIGNATURE_BYTES
        || state.signature_der.first() != Some(&0x30)
    {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    Ok(())
}

fn proof_from_bridge(
    status: i32,
    public_key: CfDataRef,
    signature: CfDataRef,
) -> Result<VoiceKeyProof, VoiceAuthorizationError> {
    if status != SUCCESS {
        release_cf_data(public_key);
        release_cf_data(signature);
        return Err(map_status(status));
    }
    let public_key_x963 = match data_from_bridge(status, public_key) {
        Ok(value) => value,
        Err(error) => {
            release_cf_data(signature);
            return Err(error);
        }
    };
    let signature_der = data_from_bridge(status, signature)?;
    validate_public_key(&public_key_x963)?;
    if signature_der.len() < 8
        || signature_der.len() > MAX_SIGNATURE_BYTES
        || signature_der[0] != 0x30
    {
        return Err(VoiceAuthorizationError::ProtocolFailure);
    }
    Ok(VoiceKeyProof {
        public_key_x963,
        signature_der,
    })
}

fn data_from_bridge(status: i32, data: CfDataRef) -> Result<Vec<u8>, VoiceAuthorizationError> {
    if status != SUCCESS {
        release_cf_data(data);
        return Err(map_status(status));
    }
    #[cfg(target_os = "macos")]
    {
        let guard = CfDataGuard(data);
        if guard.0.is_null() {
            return Err(VoiceAuthorizationError::KeyUnavailable);
        }
        // SAFETY: the native bridge retained the CFData; guard keeps it alive until copied.
        let length = usize::try_from(unsafe { CFDataGetLength(guard.0) })
            .map_err(|_| VoiceAuthorizationError::ProtocolFailure)?;
        if length == 0 || length > MAX_MESSAGE_BYTES {
            return Err(VoiceAuthorizationError::ProtocolFailure);
        }
        // SAFETY: CFDataGetBytePtr returns at least length bytes while the guard is alive.
        let pointer = unsafe { CFDataGetBytePtr(guard.0) };
        if pointer.is_null() {
            return Err(VoiceAuthorizationError::KeyUnavailable);
        }
        Ok(unsafe { slice::from_raw_parts(pointer, length) }.to_vec())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = data;
        Err(VoiceAuthorizationError::KeyUnavailable)
    }
}

#[cfg(target_os = "macos")]
struct CfDataGuard(CfDataRef);

#[cfg(target_os = "macos")]
impl Drop for CfDataGuard {
    fn drop(&mut self) {
        release_cf_data(self.0);
    }
}

fn release_cf_data(value: CfDataRef) {
    #[cfg(target_os = "macos")]
    if !value.is_null() {
        // SAFETY: this bridge releases only retained CFData it created.
        unsafe { saturday_voice_cf_release(value) };
    }
    #[cfg(not(target_os = "macos"))]
    let _ = value;
}

fn map_status(status: i32) -> VoiceAuthorizationError {
    match status {
        UNAVAILABLE | KEY_NOT_FOUND => VoiceAuthorizationError::KeyUnavailable,
        CANCELLED => VoiceAuthorizationError::OwnerCancelled,
        AUTHENTICATION_FAILED => VoiceAuthorizationError::OwnerAuthentication,
        INVALID_INPUT | BAD_SIGNATURE => VoiceAuthorizationError::ProtocolFailure,
        SYSTEM | SUCCESS => VoiceAuthorizationError::KeyUnavailable,
        _ => VoiceAuthorizationError::KeyUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_noncanonical_public_keys_before_calling_native_crypto() {
        assert!(matches!(
            validate_public_key(&[0u8; 65]),
            Err(VoiceAuthorizationError::ProtocolFailure)
        ));
        assert!(matches!(validate_public_key(&[0x04; 65]), Ok(())));
    }

    #[test]
    fn rejects_unbounded_messages_and_prompts_before_native_calls() {
        assert!(matches!(
            validate_message(&[]),
            Err(VoiceAuthorizationError::ProtocolFailure)
        ));
        assert!(matches!(
            validate_message(&vec![0u8; MAX_MESSAGE_BYTES + 1]),
            Err(VoiceAuthorizationError::ProtocolFailure)
        ));
        assert!(matches!(
            prompt_string(""),
            Err(VoiceAuthorizationError::ProtocolFailure)
        ));
        assert!(matches!(
            prompt_string("voice\0authorization"),
            Err(VoiceAuthorizationError::ProtocolFailure)
        ));
    }

    #[test]
    fn maps_biometric_cancel_and_missing_secure_enclave_without_fallback() {
        assert_eq!(
            map_status(CANCELLED),
            VoiceAuthorizationError::OwnerCancelled
        );
        assert_eq!(
            map_status(KEY_NOT_FOUND),
            VoiceAuthorizationError::KeyUnavailable
        );
        assert_eq!(
            map_status(UNAVAILABLE),
            VoiceAuthorizationError::KeyUnavailable
        );
    }

    fn signed_state() -> Vec<u8> {
        let state = saturday_voice_parent_auth::SignedAuthorizationState {
            payload: saturday_voice_parent_auth::AuthorizationStatePayload {
                schema: saturday_voice_parent_auth::AUTHORIZATION_STATE_SCHEMA.to_owned(),
                binding_sha256: "a".repeat(64),
                serial: 7,
                ledger_sha256: "b".repeat(64),
                active_assets: Vec::new(),
            },
            signature_der: vec![0x30, 6, 2, 1, 1, 2, 1, 1],
        };
        let value = serde_json::to_value(state).unwrap();
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn protected_state_requires_canonical_unique_fields_and_matching_serial() {
        let bytes = signed_state();
        assert!(validate_protected_state(&bytes, 7).is_ok());
        assert_eq!(
            validate_protected_state(&bytes, 8),
            Err(VoiceAuthorizationError::StorageInvalid),
        );
        let duplicate = String::from_utf8(bytes)
            .unwrap()
            .replace("\"serial\":7", "\"serial\":7,\"serial\":7")
            .into_bytes();
        assert_eq!(
            validate_protected_state(&duplicate, 7),
            Err(VoiceAuthorizationError::StorageInvalid),
        );
    }

    #[test]
    fn protected_state_rejects_unreviewed_receipts_and_bounded_signature_errors() {
        let mut value: serde_json::Value = serde_json::from_slice(&signed_state()).unwrap();
        value["payload"]["active_assets"] = serde_json::json!([{
            "asset_id": "unreviewed",
            "purpose": "asr_weights",
            "generation": 1,
            "version": "c".repeat(64),
            "bytes": 1,
            "sha256": "c".repeat(64),
            "state": "enabled",
            "template_fingerprint_sha256": null,
            "tokenizer_metadata_sha256": null
        }]);
        let invalid_asset = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            validate_protected_state(&invalid_asset, 7),
            Err(VoiceAuthorizationError::StorageInvalid),
        );

        value["payload"]["active_assets"] = serde_json::json!([]);
        value["signature_der"] = serde_json::json!([48, 6, 2, 1, 1, 2, 1]);
        let short_signature = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            validate_protected_state(&short_signature, 7),
            Err(VoiceAuthorizationError::StorageInvalid),
        );
    }
}
