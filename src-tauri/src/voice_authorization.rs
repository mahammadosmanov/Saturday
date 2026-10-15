use crate::voice::contract::{AssetPurpose, VoiceAssetAuthority, VoiceConsentPrompt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;
mod catalog;
mod consent;
mod gguf;
mod issuer;
mod ledger;
pub(crate) use ledger::ActiveAssetReceipt;

pub(crate) use consent::NativeVoiceConsentPrompt;
pub(crate) use issuer::{AuthorizedWorkerSession, VoiceAuthorizationService};

pub const VOICE_BINDING_SCHEMA: &str = "saturday.voice.asset-bindings.v1";
pub const VOICE_GRANT_SCHEMA: &str = "saturday.voice.transfer-grant.v1";
pub const VOICE_LEDGER_SCHEMA: &str = "saturday.voice.authorization-ledger.v1";
pub const VOICE_KEY_APPLICATION_TAG: &[u8] = b"dev.rohtolos.saturday.voice.owner-key.v1";
pub const VOICE_GRANT_DOMAIN: &[u8] = b"SATURDAY-VOICE-ASSET-GRANT-V1\0";
pub const VOICE_TRANSFER_TTL_SECONDS: u64 = 15 * 60;
pub const VOICE_ASR_MAX_RSS_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const VOICE_LLM_MAX_RSS_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const MAX_DISCLOSURE_BYTES: usize = 256 * 1024;
pub const VOICE_MODEL_PURPOSE_DOMAIN: &[u8] = b"SATURDAY-VOICE-MODEL-PURPOSE-V1\0";
pub const VOICE_MODEL_TEMPLATE_DOMAIN: &[u8] = b"SATURDAY-VOICE-MODEL-TEMPLATE-V1\0";
pub const VOICE_OWNER_BINDING_DOMAIN: &[u8] = b"SATURDAY-VOICE-OWNER-BINDING-V1\0";
pub const VOICE_PARENT_GRANT_SCHEMA: &str = "saturday.voice.parent-worker-grant.v1";
pub const VOICE_PARENT_GRANT_TTL_SECONDS: u64 = 365 * 24 * 60 * 60;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum VoiceAuthorizationError {
    #[error("voice binding is not frozen")]
    BindingUnfrozen,
    #[error("voice binding is invalid")]
    BindingInvalid,
    #[error("voice owner authorization is unavailable")]
    KeyUnavailable,
    #[error("voice owner authorization was cancelled")]
    OwnerCancelled,
    #[error("voice owner authorization failed")]
    OwnerAuthentication,
    #[error("voice asset grant is invalid")]
    GrantInvalid,
    #[error("voice transfer grant expired")]
    GrantExpired,
    #[error("voice transfer grant was already consumed")]
    GrantConsumed,
    #[error("voice transfer recovery is required")]
    RecoveryRequired,
    #[error("voice asset identity does not match the frozen binding")]
    AssetMismatch,
    #[error("voice worker identity does not match the frozen binding")]
    WorkerMismatch,
    #[error("voice model identity is invalid")]
    ModelInvalid,
    #[error("voice authorization storage is invalid")]
    StorageInvalid,
    #[error("voice authorization storage operation failed")]
    StorageFailure,
    #[error("voice authorization protocol failed")]
    ProtocolFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferOutcome {
    Promoted,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizedAssetState {
    Enabled,
    Disabled,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferResult {
    pub outcome: TransferOutcome,
    pub bytes_received: u64,
    pub sha256: String,
    pub promoted_version: Option<String>,
    pub template_fingerprint_sha256: Option<String>,
    pub tokenizer_metadata_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OwnerConsentScope {
    Download {
        asset_id: String,
    },
    ModelPurpose {
        asset_id: String,
        model_sha256: String,
        template_sha256: String,
        tokenizer_metadata_sha256: String,
    },
}

#[derive(Debug)]
pub struct OwnerConsent {
    disclosure_sha256: [u8; 32],
    caller_window: String,
    caller_origin: String,
    issued_at: Instant,
    nonce: [u8; 32],
    scope: OwnerConsentScope,
}

impl OwnerConsent {
    fn after_native_confirmation(
        disclosure_bytes: &[u8],
        caller: &crate::voice::ipc::NativeCallerContext,
        nonce: [u8; 32],
        scope: OwnerConsentScope,
    ) -> Result<Self, VoiceAuthorizationError> {
        let caller_origin = caller.origin();
        if disclosure_bytes.is_empty()
            || disclosure_bytes.len() > MAX_DISCLOSURE_BYTES
            || !caller.is_trusted()
            || !is_trusted_voice_origin(caller_origin)
            || caller.window_label() != "main"
        {
            return Err(VoiceAuthorizationError::ProtocolFailure);
        }
        Ok(Self {
            disclosure_sha256: Sha256::digest(disclosure_bytes).into(),
            caller_window: caller.window_label().to_owned(),
            caller_origin: caller_origin.to_owned(),
            issued_at: Instant::now(),
            nonce,
            scope,
        })
    }

    fn matches(
        &self,
        disclosure_bytes: &[u8],
        caller: &crate::voice::ipc::NativeCallerContext,
        scope: &OwnerConsentScope,
    ) -> bool {
        caller.is_trusted()
            && is_trusted_voice_origin(caller.origin())
            && self.caller_window == caller.window_label()
            && self.caller_origin == caller.origin()
            && &self.scope == scope
            && self.issued_at.elapsed() <= std::time::Duration::from_secs(120)
            && self.disclosure_sha256.as_slice() == Sha256::digest(disclosure_bytes).as_slice()
    }
}

fn is_trusted_voice_origin(origin: &str) -> bool {
    origin == "tauri://localhost" || {
        #[cfg(debug_assertions)]
        {
            origin == "http://127.0.0.1:1420"
        }
        #[cfg(not(debug_assertions))]
        {
            false
        }
    }
}

#[derive(Debug)]
pub struct VoiceGrant {
    pub(crate) generation: u64,
    pub(crate) grant_id: [u8; 16],
    pub(crate) asset_id: String,
    pub(crate) purpose: AssetPurpose,
    pub(crate) expected_bytes: u64,
    pub(crate) expected_sha256: String,
    pub(crate) binding_sha256: String,
    pub(crate) issued_at_unix: u64,
    pub(crate) expires_at_unix: u64,
    pub(crate) signature_der: Vec<u8>,
    pub(crate) consent_nonce: [u8; 32],
}

impl VoiceGrant {
    pub fn asset_id(&self) -> &str {
        &self.asset_id
    }

    pub fn purpose(&self) -> AssetPurpose {
        self.purpose
    }

    pub fn expected_bytes(&self) -> u64 {
        self.expected_bytes
    }

    pub fn expected_sha256(&self) -> &str {
        &self.expected_sha256
    }
}

#[derive(Debug)]
pub struct VoiceTransferLease {
    pub(crate) generation: u64,
    pub(crate) grant_id: [u8; 16],
    pub(crate) asset_id: String,
    pub(crate) purpose: AssetPurpose,
    pub(crate) expected_bytes: u64,
    pub(crate) expected_sha256: String,
    pub(crate) binding_sha256: String,
    pub(crate) expires_at_unix: u64,
}

impl VoiceTransferLease {
    pub fn asset_id(&self) -> &str {
        &self.asset_id
    }

    pub fn purpose(&self) -> AssetPurpose {
        self.purpose
    }

    pub fn expected_bytes(&self) -> u64 {
        self.expected_bytes
    }

    pub fn expected_sha256(&self) -> &str {
        &self.expected_sha256
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceModelIdentity {
    pub asset_id: String,
    pub version: String,
    pub purpose: AssetPurpose,
    pub bytes: u64,
    pub sha256: String,
    pub chat_template_sha256: Option<String>,
    pub tokenizer_metadata_sha256: Option<String>,
    pub binding_sha256: String,
}

/// Manager-produced, FD-backed identity. It is data only until the authority independently
/// verifies exact approved bytes, GGUF metadata, inode and the durable receipt.
pub struct VerifiedModelFile {
    file: File,
    asset_id: String,
    purpose: AssetPurpose,
    version: String,
    bytes: u64,
    payload_sha256: String,
    template_sha256: Option<[u8; 32]>,
    tokenizer_metadata_sha256: Option<[u8; 32]>,
    device: u64,
    inode: u64,
}

impl VerifiedModelFile {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_manager(
        file: File,
        asset_id: String,
        purpose: AssetPurpose,
        version: String,
        bytes: u64,
        payload_sha256: String,
        template_sha256: Option<[u8; 32]>,
        tokenizer_metadata_sha256: Option<[u8; 32]>,
        device: u64,
        inode: u64,
    ) -> Result<Self, VoiceAuthorizationError> {
        if bytes == 0
            || !is_sha256(&payload_sha256)
            || version != payload_sha256
            || device == 0
            || inode == 0
            || (purpose == AssetPurpose::AsrWeights
                && (template_sha256.is_some() || tokenizer_metadata_sha256.is_some()))
            || (purpose == AssetPurpose::LlmWeights
                && (template_sha256.is_none() || tokenizer_metadata_sha256.is_none()))
            || !matches!(asset_id.as_str(), "whisper-base" | "qwen3-4b-q4-k-m")
        {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        Ok(Self {
            file,
            asset_id,
            purpose,
            version,
            bytes,
            payload_sha256,
            template_sha256,
            tokenizer_metadata_sha256,
            device,
            inode,
        })
    }

    pub(crate) fn file(&self) -> &File {
        &self.file
    }
    pub(crate) fn asset_id(&self) -> &str {
        &self.asset_id
    }
    pub(crate) fn purpose(&self) -> AssetPurpose {
        self.purpose
    }
    pub(crate) fn version(&self) -> &str {
        &self.version
    }
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }
    pub(crate) fn payload_sha256(&self) -> &str {
        &self.payload_sha256
    }
    pub(crate) fn template_sha256(&self) -> Option<&[u8; 32]> {
        self.template_sha256.as_ref()
    }
    pub(crate) fn tokenizer_metadata_sha256(&self) -> Option<&[u8; 32]> {
        self.tokenizer_metadata_sha256.as_ref()
    }
}

pub struct ApprovedModelHandle {
    file: File,
    identity: VoiceModelIdentity,
    sandbox_path: PathBuf,
}

impl ApprovedModelHandle {
    fn from_verified_binding(
        file: File,
        identity: VoiceModelIdentity,
        sandbox_path: PathBuf,
    ) -> Result<Self, VoiceAuthorizationError> {
        let metadata = file
            .metadata()
            .map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
        if !metadata.is_file()
            || metadata.len() != identity.bytes
            || identity.bytes == 0
            || !is_sha256(&identity.sha256)
            || !is_sha256(&identity.binding_sha256)
            || (identity.purpose == AssetPurpose::LlmWeights
                && (identity
                    .chat_template_sha256
                    .as_deref()
                    .is_none_or(|hash| !is_sha256(hash))
                    || identity
                        .tokenizer_metadata_sha256
                        .as_deref()
                        .is_none_or(|hash| !is_sha256(hash))))
            || (identity.purpose != AssetPurpose::LlmWeights
                && (identity.chat_template_sha256.is_some()
                    || identity.tokenizer_metadata_sha256.is_some()))
        {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        Ok(Self {
            file,
            identity,
            sandbox_path,
        })
    }

    pub fn as_file(&self) -> &File {
        &self.file
    }

    pub fn into_file(self) -> File {
        self.file
    }

    pub fn raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    pub fn size_bytes(&self) -> u64 {
        self.identity.bytes
    }

    pub fn purpose(&self) -> AssetPurpose {
        self.identity.purpose
    }

    pub fn asset_id(&self) -> &str {
        &self.identity.asset_id
    }

    pub fn sha256(&self) -> &str {
        &self.identity.sha256
    }

    pub fn chat_template_sha256(&self) -> Option<&str> {
        self.identity.chat_template_sha256.as_deref()
    }
    pub fn tokenizer_metadata_sha256(&self) -> Option<&str> {
        self.identity.tokenizer_metadata_sha256.as_deref()
    }

    pub(crate) fn sandbox_read_path(&self) -> &Path {
        &self.sandbox_path
    }

    pub(crate) fn binding_sha256(&self) -> &str {
        &self.identity.binding_sha256
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRole {
    Asr,
    Llm,
}

pub struct ApprovedRuntimeClosure {
    binding_sha256: String,
    role: WorkerRole,
    executable: File,
    executable_path: PathBuf,
    worker_sha256: String,
    worker_bytes: u64,
    worker_bundle_identifier: String,
    worker_team_id_sha256: String,
    worker_requirement_sha256: String,
    source_seal_sha256: String,
    resource_seal_sha256: String,
    max_rss_bytes: u64,
    parent_grant: saturday_voice_parent_auth::ParentWorkerGrant,
    authorization_state: Vec<u8>,
    parent_resource_files: Vec<PathBuf>,
}

pub(crate) struct RuntimeClosureBinding {
    pub(crate) role: WorkerRole,
    pub(crate) executable: File,
    pub(crate) executable_path: PathBuf,
    pub(crate) worker_sha256: String,
    pub(crate) worker_bytes: u64,
    pub(crate) worker_bundle_identifier: String,
    pub(crate) worker_team_id_sha256: String,
    pub(crate) worker_requirement_sha256: String,
    pub(crate) source_seal_sha256: String,
    pub(crate) resource_seal_sha256: String,
    pub(crate) max_rss_bytes: u64,
    pub(crate) binding_sha256: String,
    pub(crate) parent_grant: saturday_voice_parent_auth::ParentWorkerGrant,
    pub(crate) authorization_state: Vec<u8>,
    pub(crate) parent_resource_files: Vec<PathBuf>,
}

impl ApprovedRuntimeClosure {
    fn from_verified_binding(
        binding: RuntimeClosureBinding,
    ) -> Result<Self, VoiceAuthorizationError> {
        let RuntimeClosureBinding {
            role,
            executable,
            executable_path,
            worker_sha256,
            worker_bytes,
            worker_bundle_identifier,
            worker_team_id_sha256,
            worker_requirement_sha256,
            source_seal_sha256,
            resource_seal_sha256,
            max_rss_bytes,
            binding_sha256,
            parent_grant,
            authorization_state,
            parent_resource_files,
        } = binding;
        let metadata = executable
            .metadata()
            .map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        let ceiling = match role {
            WorkerRole::Asr => VOICE_ASR_MAX_RSS_BYTES,
            WorkerRole::Llm => VOICE_LLM_MAX_RSS_BYTES,
        };
        if !metadata.is_file()
            || metadata.len() != worker_bytes
            || worker_bytes == 0
            || max_rss_bytes == 0
            || max_rss_bytes > ceiling
            || !is_sha256(&worker_sha256)
            || !is_sha256(&worker_team_id_sha256)
            || !is_sha256(&worker_requirement_sha256)
            || worker_bundle_identifier.is_empty()
            || worker_bundle_identifier.len() > 255
            || !is_sha256(&source_seal_sha256)
            || !is_sha256(&resource_seal_sha256)
            || !is_sha256(&binding_sha256)
            || authorization_state.is_empty()
            || authorization_state.len() > 8_192
            || parent_resource_files.len() < 4
            || parent_resource_files.len() > 132
            || parent_resource_files.iter().any(|path| !path.is_absolute())
            || parent_resource_files
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != parent_resource_files.len()
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        Ok(Self {
            role,
            executable,
            executable_path,
            worker_sha256,
            worker_bytes,
            worker_bundle_identifier,
            worker_team_id_sha256,
            worker_requirement_sha256,
            source_seal_sha256,
            resource_seal_sha256,
            max_rss_bytes,
            binding_sha256,
            parent_grant,
            authorization_state,
            parent_resource_files,
        })
    }

    pub fn role(&self) -> WorkerRole {
        self.role
    }
    pub fn worker_bundle_identifier(&self) -> &str {
        &self.worker_bundle_identifier
    }

    pub fn worker_team_id_sha256(&self) -> &str {
        &self.worker_team_id_sha256
    }

    pub fn worker_requirement_sha256(&self) -> &str {
        &self.worker_requirement_sha256
    }

    pub fn executable(&self) -> &Path {
        &self.executable_path
    }

    pub fn worker_sha256(&self) -> &str {
        &self.worker_sha256
    }

    pub fn worker_bytes(&self) -> u64 {
        self.worker_bytes
    }

    pub fn source_seal_sha256(&self) -> &str {
        &self.source_seal_sha256
    }

    pub fn resource_seal_sha256(&self) -> &str {
        &self.resource_seal_sha256
    }

    pub fn max_rss_bytes(&self) -> u64 {
        self.max_rss_bytes
    }

    pub fn executable_file(&self) -> &File {
        &self.executable
    }

    pub(crate) fn binding_sha256(&self) -> &str {
        &self.binding_sha256
    }
    pub(crate) fn parent_grant_template(&self) -> &saturday_voice_parent_auth::ParentWorkerGrant {
        &self.parent_grant
    }
    pub fn parent_resource_files(&self) -> &[PathBuf] {
        &self.parent_resource_files
    }

    pub(crate) fn validate_current_authorization_state(
        &self,
    ) -> Result<(), VoiceAuthorizationError> {
        let current = crate::platform::voice_authorization::read_protected_authorization_state()?
            .ok_or(VoiceAuthorizationError::StorageInvalid)?;
        if current != self.authorization_state {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VoiceOwnerBootstrapReview {
    pub owner_public_key_sha256: String,
    pub application_bundle_identifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VoiceOwnerBootstrapResult {
    BootstrapAnchorCreated(VoiceOwnerBootstrapReview),
    FinalBinding(VoiceBindingReview),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkerAuthorizationNeeds {
    pub(crate) asr: bool,
    pub(crate) llm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceBindingReview {
    pub binding_sha256: String,
    pub key_id_sha256: String,
    pub reviewer_roster_sha256: String,
}

pub trait VoiceKeySession: Send + Sync {
    fn sign(&self, message: &[u8]) -> Result<VoiceKeyProof, VoiceAuthorizationError>;
    fn invalidate(&self);
    fn is_valid(&self) -> bool;
}

pub trait VoiceOwnerKey: Send + Sync {
    fn provision(
        &self,
        challenge: &[u8],
        prompt: &str,
    ) -> Result<VoiceKeyProof, VoiceAuthorizationError>;
    fn public_key(&self, prompt: &str) -> Result<Vec<u8>, VoiceAuthorizationError>;
    fn sign(&self, message: &[u8], prompt: &str) -> Result<VoiceKeyProof, VoiceAuthorizationError>;
    fn preauthorize(
        &self,
        prompt: &str,
    ) -> Result<Arc<dyn VoiceKeySession>, VoiceAuthorizationError>;
    fn verify(
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<bool, VoiceAuthorizationError>
    where
        Self: Sized;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceKeyProof {
    pub public_key_x963: Vec<u8>,
    pub signature_der: Vec<u8>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn current_unix_seconds() -> Result<u64, VoiceAuthorizationError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| VoiceAuthorizationError::StorageFailure)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn purpose_matches_worker(purpose: AssetPurpose, role: WorkerRole) -> bool {
    matches!(
        (purpose, role),
        (AssetPurpose::AsrWeights, WorkerRole::Asr) | (AssetPurpose::LlmWeights, WorkerRole::Llm)
    )
}

// These imports intentionally pin the authorization types to the shared native trait without
// exposing any grant or file handle to public IPC serialization.
const _: Option<&dyn VoiceAssetAuthority> = None;
const _: Option<&dyn VoiceConsentPrompt> = None;

#[cfg(all(test, target_os = "macos"))]
mod crypto_tests {
    use super::VoiceOwnerKey;

    fn from_hex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                let byte = std::str::from_utf8(pair).expect("ASCII hex");
                u8::from_str_radix(byte, 16).expect("valid hex")
            })
            .collect()
    }

    #[test]
    fn verifies_real_p256_signature_and_rejects_mutation() {
        // RFC 6979 A.2.5, P-256, SHA-256("sample").
        let mut public_key = vec![0x04];
        public_key.extend(from_hex(
            "60fed4ba255a9d31c961eb74c6356d68c049b8923b61fa6ce669622e60f29fb6\
             7903fe1008b8bc99a41ae9e95628bc64f2f1b20c2d7e9f5177a3c294d4462299",
        ));
        let mut signature = from_hex(
            "3046022100efd48b2aacb6a8fd1140dd9cd45e81d69d2c877b56aaf991c34d0ea84eaf3716\
             022100f7cb1c942d657c41d436c7a1b6e29f65f3e900dbb9aff4064dc4ab2f843acda8",
        );
        assert!(<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(&public_key, b"sample", &signature).unwrap());
        let last = signature.last_mut().expect("DER signature");
        *last ^= 1;
        assert!(!<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(&public_key, b"sample", &signature).unwrap());
    }
}
