use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::os::unix::fs::MetadataExt;

pub const OWNER_KEY_APPLICATION_TAG: &[u8] = b"dev.rohtolos.saturday.voice.owner-key.v1";
pub const OWNER_KEY_BOOTSTRAP_DOMAIN: &[u8] = b"SATURDAY-VOICE-OWNER-KEY-BOOTSTRAP-V1\0";
pub const OWNER_KEY_BOOTSTRAP_SCHEMA: &str = "saturday.voice.owner-key-bootstrap.v1";
pub const OWNER_KEY_PIN_SCHEMA: &str = "saturday.voice.owner-key-pin.v1";
pub const PARENT_GRANT_SCHEMA: &str = "saturday.voice.parent-worker-grant.v3";
pub const WORKER_LAUNCH_AUTH_DOMAIN: &[u8] = b"SATURDAY-VOICE-WORKER-LAUNCH-V1\0";
pub const MAX_RUNTIME_MANIFEST_BYTES: usize = 262_144;
pub const MAX_RUNTIME_RESOURCES: usize = 128;
pub const MAX_RUNTIME_WORKERS: usize = 2;
const MAX_PARENT_GRANT_BYTES: usize = 8_192;
const OWNER_BINDING_DOMAIN: &[u8] = b"SATURDAY-VOICE-OWNER-BINDING-V1\0";
const GRANT_DOMAIN: &[u8] = b"SATURDAY-VOICE-ASSET-GRANT-V1\0";
const MODEL_PURPOSE_DOMAIN: &[u8] = b"SATURDAY-VOICE-MODEL-PURPOSE-V1\0";
const RESOURCE_CLOSURE_DOMAIN: &[u8] = b"SATURDAY-VOICE-RESOURCE-CLOSURE-V1\0";
pub const MAX_PARENT_AUTH_TTL_SECONDS: u64 = 365 * 24 * 60 * 60;
const MAX_SIGNATURE_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 4096;
const MAX_BOOTSTRAP_RECORD_BYTES: usize = 8192;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerKeyBootstrapPayload {
    pub schema: String,
    pub application_tag_sha256: String,
    pub owner_public_key_x963: Vec<u8>,
    pub owner_public_key_sha256: String,
    pub application_bundle_identifier: String,
    pub application_team_id_sha256: String,
    pub application_requirement_sha256: String,
    pub issued_at_unix: u64,
    pub nonce_sha256: String,
}

pub fn verify_owner_key_bootstrap_record(bytes: &[u8]) -> Result<Vec<u8>, ParentAuthError> {
    if bytes.is_empty() || bytes.len() > MAX_BOOTSTRAP_RECORD_BYTES {
        return Err(ParentAuthError::InvalidGrant);
    }
    let signed: Signed<OwnerKeyBootstrapPayload> =
        serde_json::from_slice(bytes).map_err(|_| ParentAuthError::InvalidGrant)?;
    if canonical_json(&signed)? != bytes
        || signed.payload.schema != OWNER_KEY_BOOTSTRAP_SCHEMA
        || signed.payload.application_tag_sha256 != sha256_hex(OWNER_KEY_APPLICATION_TAG)
        || signed.payload.owner_public_key_x963.len() != 65
        || signed.payload.owner_public_key_x963[0] != 0x04
        || signed.payload.owner_public_key_sha256 != sha256_hex(&signed.payload.owner_public_key_x963)
        || !valid_sha256(&signed.payload.application_team_id_sha256)
        || !valid_sha256(&signed.payload.application_requirement_sha256)
        || !valid_sha256(&signed.payload.nonce_sha256)
        || signed.payload.application_bundle_identifier.is_empty()
        || signed.payload.application_bundle_identifier.len() > 255
        || signed.payload.issued_at_unix == 0
        || signed.signature_der.len() < 8
        || signed.signature_der.len() > MAX_SIGNATURE_BYTES
        || signed.signature_der.first() != Some(&0x30)
    {
        return Err(ParentAuthError::InvalidGrant);
    }
    if !verify_signed_payload(
        OWNER_KEY_BOOTSTRAP_DOMAIN,
        &signed.payload,
        &signed.signature_der,
        &signed.payload.owner_public_key_x963,
    )? {
        return Err(ParentAuthError::InvalidSignature);
    }
    Ok(signed.payload.owner_public_key_x963)
}
pub const OWNER_KEY_BOOTSTRAP_WITNESS_DOMAIN: &[u8] = b"SATURDAY-VOICE-OWNER-KEY-BOOTSTRAP-WITNESS-V1\0";
pub const OWNER_KEY_BOOTSTRAP_WITNESS_SCHEMA: &str = "saturday.voice.owner-key-bootstrap-witness.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerKeyBootstrapWitnessPayload {
    pub schema: String,
    pub application_tag_sha256: String,
    pub owner_public_key_sha256: String,
    pub bootstrap_record_sha256: String,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
    pub nonce_sha256: String,
}

pub fn verify_owner_key_bootstrap_witness(
    witness_bytes: &[u8],
    bootstrap_record_bytes: &[u8],
) -> Result<(), ParentAuthError> {
    if witness_bytes.is_empty() || witness_bytes.len() > 8192 {
        return Err(ParentAuthError::InvalidGrant);
    }
    let owner_public_key = verify_owner_key_bootstrap_record(bootstrap_record_bytes)?;
    let signed: Signed<OwnerKeyBootstrapWitnessPayload> =
        serde_json::from_slice(witness_bytes).map_err(|_| ParentAuthError::InvalidGrant)?;
    let now = now_unix_seconds()?;
    let payload = &signed.payload;
    if canonical_json(&signed)? != witness_bytes
        || payload.schema != OWNER_KEY_BOOTSTRAP_WITNESS_SCHEMA
        || payload.application_tag_sha256 != sha256_hex(OWNER_KEY_APPLICATION_TAG)
        || payload.owner_public_key_sha256 != sha256_hex(&owner_public_key)
        || payload.bootstrap_record_sha256 != sha256_hex(bootstrap_record_bytes)
        || payload.issued_at_unix == 0
        || payload.issued_at_unix > now
        || payload.expires_at_unix <= now
        || payload.expires_at_unix.saturating_sub(payload.issued_at_unix) > 120
        || !valid_sha256(&payload.nonce_sha256)
        || signed.signature_der.len() < 8
        || signed.signature_der.len() > MAX_SIGNATURE_BYTES
        || signed.signature_der.first() != Some(&0x30)
    {
        return Err(ParentAuthError::InvalidGrant);
    }
    if !verify_signed_payload(
        OWNER_KEY_BOOTSTRAP_WITNESS_DOMAIN,
        payload,
        &signed.signature_der,
        &owner_public_key,
    )? {
        return Err(ParentAuthError::InvalidSignature);
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRole {
    Asr,
    Llm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentAuthError {
    InvalidGrant,
    InvalidSignature,
    SourceReviewRequired,
    ParentIdentityMismatch,
    WorkerIdentityMismatch,
    ResourceClosureMismatch,
    Expired,
    PlatformUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed<T> {
    pub payload: T,
    pub signature_der: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentWorkerGrant {
    pub schema: String,
    pub worker_role: WorkerRole,
    pub owner_binding: Signed<OwnerBindingPayload>,
    pub asset_grant: Signed<TransferGrantPayload>,
    pub model_purpose: Option<Signed<ModelPurposePayload>>,
    pub authorization_state: SignedAuthorizationState,
    pub launch_authorization: Option<Signed<WorkerLaunchAuthorizationPayload>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerLaunchAuthorizationPayload {
    pub schema: String,
    pub challenge_hex: String,
    pub child_pid: u32,
    pub child_unique_id: u64,
    pub worker_role: WorkerRole,
    pub asset_id: String,
    pub generation: u64,
    pub binding_sha256: String,
    pub owner_public_key_sha256: String,
    pub parent_executable_sha256: String,
    pub parent_resource_closure_sha256: String,
    pub authorization_state_serial: u64,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
    pub nonce_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerBindingPayload {
    pub schema: String,
    pub binding_sha256: String,
    pub key_id_sha256: String,
    pub owner_public_key_sha256: String,
    pub reviewer_roster_sha256: String,
    pub owner_public_key_x963: Vec<u8>,
    pub generation: u64,
    pub application_bundle_identifier: String,
    pub application_team_id_sha256: String,
    pub application_requirement_sha256: String,
    pub parent_executable_sha256: String,
    pub parent_resource_closure_sha256: String,
    pub resource_manifest_sha256: String,
    pub workers: Vec<WorkerBinding>,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
    pub nonce_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerBinding {
    pub source_revision: String,
    pub role: WorkerRole,
    pub asset_id: String,
    pub purpose: String,
    pub model_sha256: String,
    pub model_bytes: u64,
    pub source_generation_sha256: String,
    pub source_tree_sha256: String,
    pub resource_generation_sha256: String,
    pub worker_sha256: String,
    pub worker_bytes: u64,
    pub worker_team_id_sha256: String,
    pub worker_requirement_sha256: String,
    pub source_seal_sha256: String,
    pub resource_seal_sha256: String,
    pub max_rss_bytes: u64,
    pub template_fingerprint_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferGrantPayload {
    pub schema: String,
    pub request_role: String,
    pub generation: u64,
    pub grant_id_hex: String,
    pub asset_id: String,
    pub purpose: String,
    pub worker_role: WorkerRole,
    pub expected_bytes: u64,
    pub expected_sha256: String,
    pub binding_sha256: String,
    pub source_generation_sha256: String,
    pub source_tree_sha256: String,
    pub resource_generation_sha256: String,
    pub worker_sha256: String,
    pub worker_bytes: u64,
    pub worker_source_seal_sha256: String,
    pub worker_resource_seal_sha256: String,
    pub template_fingerprint_sha256: Option<String>,
    pub application_bundle_identifier: String,
    pub application_team_id_sha256: String,
    pub application_requirement_sha256: String,
    pub parent_executable_sha256: String,
    pub parent_resource_closure_sha256: String,
    pub worker_team_id_sha256: String,
    pub worker_requirement_sha256: String,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
    pub use_expires_at_unix: u64,
    pub consent_nonce_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPurposePayload {
    pub schema: String,
    pub serial: u64,
    pub asset_id: String,
    pub model_sha256: String,
    pub template_sha256: String,
    pub tokenizer_metadata_sha256: String,
    pub architecture: String,
    pub context_length: u32,
    pub binding_sha256: String,
    pub source_generation_sha256: String,
    pub source_tree_sha256: String,
    pub resource_generation_sha256: String,
    pub worker_sha256: String,
    pub worker_source_seal_sha256: String,
    pub worker_resource_seal_sha256: String,
    pub application_bundle_identifier: String,
    pub application_team_id_sha256: String,
    pub application_requirement_sha256: String,
    pub parent_executable_sha256: String,
    pub parent_resource_closure_sha256: String,
    pub worker_team_id_sha256: String,
    pub worker_requirement_sha256: String,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
    pub consent_nonce_sha256: String,
}

pub const AUTHORIZATION_STATE_DOMAIN: &[u8] = b"SATURDAY-VOICE-AUTHORIZATION-STATE-V1\0";
pub const AUTHORIZATION_STATE_SCHEMA: &str = "saturday.voice.authorization-state.v1";
pub const MAX_AUTHORIZATION_STATE_BYTES: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationAssetState {
    pub asset_id: String,
    pub purpose: String,
    pub generation: u64,
    pub version: String,
    pub bytes: u64,
    pub sha256: String,
    pub state: String,
    pub template_fingerprint_sha256: Option<String>,
    pub tokenizer_metadata_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationStatePayload {
    pub schema: String,
    pub binding_sha256: String,
    pub serial: u64,
    pub ledger_sha256: String,
    pub active_assets: Vec<AuthorizationAssetState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedAuthorizationState {
    pub payload: AuthorizationStatePayload,
    pub signature_der: Vec<u8>,
}
#[derive(Debug, Clone, Copy)]
pub struct ParentWorkerGrantContext<'a> {
    pub observed_parent_pid: libc::pid_t,
    pub role: WorkerRole,
    pub worker_sha256: &'a str,
    pub source_seal_sha256: &'a str,
    pub resource_seal_sha256: &'a str,
    pub trusted_owner_public_key_x963: &'a [u8],
    pub expected_challenge_hex: &'a str,
}

#[derive(Clone, Copy)]
struct AssetGrantExpectation<'a> {
    asset_id: &'a str,
    purpose: &'a str,
    generation: u64,
    expected_bytes: u64,
    expected_sha256: &'a str,
    role: WorkerRole,
    model_purpose: Option<&'a ModelPurposePayload>,
}

struct LiveCodeIdentity {
    team: Vec<u8>,
    identifier: Vec<u8>,
    requirement: Vec<u8>,
}
struct WorkerLaunchVerificationContext<'a> {
    expected_challenge_hex: &'a str,
    role: WorkerRole,
    transfer: &'a TransferGrantPayload,
    owner: &'a OwnerBindingPayload,
    authorization_state_serial: u64,
    now: u64,
    trusted_owner_public_key_x963: &'a [u8],
}

impl AuthorizationStatePayload {
    pub fn validate(&self) -> Result<(), ParentAuthError> {
        if self.schema != AUTHORIZATION_STATE_SCHEMA
            || !valid_sha256(&self.binding_sha256)
            || !valid_sha256(&self.ledger_sha256)
            || self.active_assets.len() > 2
            || self.active_assets.windows(2).any(|pair| pair[0].asset_id >= pair[1].asset_id)
        {
            return Err(ParentAuthError::InvalidGrant);
        }
        for asset in &self.active_assets {
            let (purpose, bytes, sha256, role) = match asset.asset_id.as_str() {
                "whisper-base" => (
                    "asr_weights",
                    147_951_465,
                    "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
                    WorkerRole::Asr,
                ),
                "qwen3-4b-q4-k-m" => (
                    "llm_weights",
                    2_497_280_256,
                    "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5",
                    WorkerRole::Llm,
                ),
                _ => return Err(ParentAuthError::InvalidGrant),
            };
            if asset.generation == 0
                || asset.generation > self.serial
                || asset.purpose != purpose
                || asset.version != asset.sha256
                || asset.bytes != bytes
                || asset.sha256 != sha256
                || !matches!(asset.state.as_str(), "enabled" | "disabled" | "revoked")
                || (role == WorkerRole::Asr
                    && (asset.template_fingerprint_sha256.is_some()
                        || asset.tokenizer_metadata_sha256.is_some()))
                || (role == WorkerRole::Llm
                    && (asset.template_fingerprint_sha256.as_deref().is_none_or(|hash| !valid_sha256(hash))
                        || asset.tokenizer_metadata_sha256.as_deref().is_none_or(|hash| !valid_sha256(hash))))
            {
                return Err(ParentAuthError::InvalidGrant);
            }
        }
        Ok(())
    }
}

fn verify_authorization_state(
    bytes: &[u8],
    owner_public_key: &[u8],
    binding_sha256: &str,
) -> Result<AuthorizationStatePayload, ParentAuthError> {
    if bytes.is_empty() || bytes.len() > MAX_AUTHORIZATION_STATE_BYTES {
        return Err(ParentAuthError::InvalidGrant);
    }
    let signed: SignedAuthorizationState =
        serde_json::from_slice(bytes).map_err(|_| ParentAuthError::InvalidGrant)?;
    if canonical_json(&signed)? != bytes {
        return Err(ParentAuthError::InvalidGrant);
    }
    signed.payload.validate()?;
    if signed.payload.binding_sha256 != binding_sha256
        || !verify_signed_payload(
            AUTHORIZATION_STATE_DOMAIN, &signed.payload, &signed.signature_der, owner_public_key,
        )?
    {
        return Err(ParentAuthError::InvalidSignature);
    }
    Ok(signed.payload)
}

fn verify_state_authorizes_grant(
    state: &AuthorizationStatePayload,
    transfer: &TransferGrantPayload,
    purpose: Option<&ModelPurposePayload>,
) -> Result<(), ParentAuthError> {
    let active = state.active_assets.iter().find(|asset| asset.asset_id == transfer.asset_id)
        .ok_or(ParentAuthError::InvalidGrant)?;
    if !asset_state_matches_grant(active, AssetGrantExpectation {
        asset_id: &transfer.asset_id,
        purpose: &transfer.purpose,
        generation: transfer.generation,
        expected_bytes: transfer.expected_bytes,
        expected_sha256: &transfer.expected_sha256,
        role: transfer.worker_role,
        model_purpose: purpose,
    }) {
        return Err(ParentAuthError::InvalidGrant);
    }
    Ok(())
}

fn asset_state_matches_grant(
    active: &AuthorizationAssetState,
    expected: AssetGrantExpectation<'_>,
) -> bool {
    if active.state != "enabled"
        || active.asset_id != expected.asset_id
        || active.generation != expected.generation
        || active.purpose != expected.purpose
        || active.version != expected.expected_sha256
        || active.bytes != expected.expected_bytes
        || active.sha256 != expected.expected_sha256
    {
        return false;
    }
    match (expected.role, expected.model_purpose) {
        (WorkerRole::Asr, None) => active.template_fingerprint_sha256.is_none()
            && active.tokenizer_metadata_sha256.is_none(),
        (WorkerRole::Llm, Some(purpose)) => {
            active.template_fingerprint_sha256.as_deref() == Some(purpose.template_sha256.as_str())
                && active.tokenizer_metadata_sha256.as_deref()
                    == Some(purpose.tokenizer_metadata_sha256.as_str())
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    pub schema: String,
    pub source_generations: Vec<SourceGenerationRecord>,
    pub workers: Vec<RuntimeWorkerRecord>,
    pub resources: Vec<RuntimeResourceRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceGenerationRecord {
    pub role: WorkerRole,
    pub source_revision: String,
    pub source_tree_sha256: String,
    pub source_generation_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeWorkerRecord {
    pub role: WorkerRole,
    pub asset_id: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub team_id_sha256: String,
    pub designated_requirement_sha256: String,
    pub source_seal_sha256: String,
    pub resource_seal_sha256: String,
    pub max_rss_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeResourceRecord {
    pub path: String,
    pub bytes: u64,
    pub mode: u32,
    pub sha256: String,
}

pub fn verify_parent_worker_grant(
    bytes: &[u8],
    context: ParentWorkerGrantContext<'_>,
) -> Result<(), ParentAuthError> {
    let ParentWorkerGrantContext {
        observed_parent_pid, role, worker_sha256, source_seal_sha256,
        resource_seal_sha256, trusted_owner_public_key_x963, expected_challenge_hex,
    } = context;
    if bytes.is_empty()
        || bytes.len() > MAX_PARENT_GRANT_BYTES
        || observed_parent_pid <= 0
        || observed_parent_pid != unsafe { libc::getppid() }
    {
        return Err(ParentAuthError::InvalidGrant);
    }
    let grant: ParentWorkerGrant = serde_json::from_slice(bytes).map_err(|_| ParentAuthError::InvalidGrant)?;
    if canonical_json(&grant)? != bytes || grant.schema != PARENT_GRANT_SCHEMA || grant.worker_role != role {
        return Err(ParentAuthError::InvalidGrant);
    }
    let owner = &grant.owner_binding.payload;
    if trusted_owner_public_key_x963.len() != 65
        || trusted_owner_public_key_x963.first() != Some(&0x04)
        || owner.owner_public_key_x963.as_slice() != trusted_owner_public_key_x963
        || sha256_hex(trusted_owner_public_key_x963) != owner.owner_public_key_sha256
    {
        return Err(ParentAuthError::InvalidGrant);
    }
    let owner_public_key = trusted_owner_public_key_x963;
    validate_owner_binding(owner)?;
    if owner.key_id_sha256 != key_id_sha256()
        || !verify_signed(OWNER_BINDING_DOMAIN, &grant.owner_binding, owner_public_key)?
    {
        return Err(ParentAuthError::InvalidSignature);
    }
    let now = now_unix_seconds()?;
    if now > owner.expires_at_unix {
        return Err(ParentAuthError::Expired);
    }
    verify_live_parent(observed_parent_pid, owner)?;
    verify_runtime_resources(owner)?;
    let binding = owner.workers.iter().find(|item| item.role == role).ok_or(ParentAuthError::InvalidGrant)?;
    if binding.worker_sha256 != worker_sha256
        || binding.source_seal_sha256 != source_seal_sha256
        || binding.resource_seal_sha256 != resource_seal_sha256
    {
        return Err(ParentAuthError::WorkerIdentityMismatch);
    }
    let transfer = &grant.asset_grant.payload;
    validate_transfer(transfer, owner, binding, role, now)?;
    if !verify_signed(GRANT_DOMAIN, &grant.asset_grant, owner_public_key)? {
        return Err(ParentAuthError::InvalidSignature);
    }
    match (role, grant.model_purpose.as_ref()) {
        (WorkerRole::Asr, None) => {}
        (WorkerRole::Asr, Some(_)) => return Err(ParentAuthError::InvalidGrant),
        (WorkerRole::Llm, Some(purpose)) => {
            validate_model_purpose(&purpose.payload, transfer, owner, binding, now)?;
            if !verify_signed(MODEL_PURPOSE_DOMAIN, purpose, owner_public_key)? {
                return Err(ParentAuthError::InvalidSignature);
            }
        }
        (WorkerRole::Llm, None) => return Err(ParentAuthError::InvalidGrant),
    }
    let authorization_state_bytes = canonical_json(&grant.authorization_state)?;
    if authorization_state_bytes.len() > MAX_AUTHORIZATION_STATE_BYTES {
        return Err(ParentAuthError::InvalidGrant);
    }
    let authorization_state = verify_authorization_state(
        &authorization_state_bytes, owner_public_key, &owner.binding_sha256,
    )?;
    let launch_authorization = grant.launch_authorization.as_ref().ok_or(ParentAuthError::InvalidGrant)?;
    verify_worker_launch_authorization(launch_authorization, WorkerLaunchVerificationContext {
        expected_challenge_hex,
        role,
        transfer,
        owner,
        authorization_state_serial: authorization_state.serial,
        now,
        trusted_owner_public_key_x963: owner_public_key,
    })?;
    verify_state_authorizes_grant(
        &authorization_state,
        transfer,
        grant.model_purpose.as_ref().map(|purpose| &purpose.payload),
    )?;
    verify_live_worker(role, binding, worker_sha256)?;
    Ok(())
}


fn verify_worker_launch_authorization(
    signed: &Signed<WorkerLaunchAuthorizationPayload>,
    context: WorkerLaunchVerificationContext<'_>,
) -> Result<(), ParentAuthError> {
    let WorkerLaunchVerificationContext {
        expected_challenge_hex, role, transfer, owner, authorization_state_serial, now,
        trusted_owner_public_key_x963,
    } = context;
    let launch = &signed.payload;
    if !valid_sha256(expected_challenge_hex)
        || launch.schema != "saturday.voice.worker-launch-authorization.v1"
        || launch.challenge_hex != expected_challenge_hex
        || launch.child_pid != unsafe { libc::getpid() } as u32
        || launch.child_unique_id == 0
        || launch.worker_role != role
        || launch.asset_id != transfer.asset_id
        || launch.generation != transfer.generation
        || launch.binding_sha256 != owner.binding_sha256
        || launch.owner_public_key_sha256 != owner.owner_public_key_sha256
        || launch.parent_executable_sha256 != owner.parent_executable_sha256
        || launch.parent_resource_closure_sha256 != owner.parent_resource_closure_sha256
        || launch.authorization_state_serial != authorization_state_serial
        || launch.issued_at_unix > now
        || launch.expires_at_unix <= now
        || launch.expires_at_unix.saturating_sub(launch.issued_at_unix) > 60
        || !valid_sha256(&launch.nonce_sha256)
    {
        return Err(ParentAuthError::InvalidGrant);
    }
    if current_process_unique_id()? != launch.child_unique_id {
        return Err(ParentAuthError::InvalidGrant);
    }
    if !verify_signed(WORKER_LAUNCH_AUTH_DOMAIN, signed, trusted_owner_public_key_x963)? {
        return Err(ParentAuthError::InvalidSignature);
    }
    Ok(())
}
pub fn verify_owner_binding_for_process(
    owner: &OwnerBindingPayload,
    pid: libc::pid_t,
) -> Result<(), ParentAuthError> {
    owner.validate()?;
    if pid <= 0 {
        return Err(ParentAuthError::ParentIdentityMismatch);
    }
    verify_live_parent(pid, owner)?;
    verify_runtime_resources(owner)
}

impl OwnerBindingPayload {
    pub fn validate(&self) -> Result<(), ParentAuthError> {
        if self.schema != "saturday.voice.owner-binding.v1"
            || !valid_sha256(&self.binding_sha256)
            || !valid_sha256(&self.key_id_sha256)
            || !valid_sha256(&self.owner_public_key_sha256)
            || !valid_sha256(&self.reviewer_roster_sha256)
            || self.generation == 0
            || !valid_bundle_id(&self.application_bundle_identifier)
            || !valid_sha256(&self.application_team_id_sha256)
            || !valid_sha256(&self.application_requirement_sha256)
            || !valid_sha256(&self.parent_executable_sha256)
            || !valid_sha256(&self.parent_resource_closure_sha256)
            || self.owner_public_key_x963.len() != 65
            || self.owner_public_key_x963.first() != Some(&0x04)
            || sha256_hex(&self.owner_public_key_x963) != self.owner_public_key_sha256
            || !valid_sha256(&self.resource_manifest_sha256)
            || self.workers.len() != MAX_RUNTIME_WORKERS
            || self.issued_at_unix == 0
            || self.expires_at_unix <= self.issued_at_unix
            || self.expires_at_unix - self.issued_at_unix > MAX_PARENT_AUTH_TTL_SECONDS
            || !valid_sha256(&self.nonce_sha256)
        {
            return Err(ParentAuthError::InvalidGrant);
        }
        let mut roles = std::collections::HashSet::new();
        for worker in &self.workers {
            validate_worker_binding(worker)?;
            if !roles.insert(worker.role) {
                return Err(ParentAuthError::InvalidGrant);
            }
        }
        if !roles.contains(&WorkerRole::Asr) || !roles.contains(&WorkerRole::Llm) {
            return Err(ParentAuthError::InvalidGrant);
        }
        Ok(())
    }
}

fn validate_worker_binding(value: &WorkerBinding) -> Result<(), ParentAuthError> {
    let (asset_id, purpose) = match value.role {
        WorkerRole::Asr => ("whisper-base", "asr_weights"),
        WorkerRole::Llm => ("qwen3-4b-q4-k-m", "llm_weights"),
    };
    if value.source_revision.len() != 40
        || !value.source_revision.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || value.asset_id != asset_id
        || value.purpose != purpose
        || !valid_sha256(&value.model_sha256)
        || value.model_bytes == 0
        || !valid_sha256(&value.source_generation_sha256)
        || !valid_sha256(&value.source_tree_sha256)
        || !valid_sha256(&value.resource_generation_sha256)
        || !valid_sha256(&value.worker_sha256)
        || value.worker_bytes == 0
        || !valid_sha256(&value.worker_team_id_sha256)
        || !valid_sha256(&value.worker_requirement_sha256)
        || !valid_sha256(&value.source_seal_sha256)
        || !valid_sha256(&value.resource_seal_sha256)
        || value.max_rss_bytes == 0
        || value.template_fingerprint_sha256.as_deref().is_some_and(|value| !valid_sha256(value))
        || (value.role == WorkerRole::Asr && value.template_fingerprint_sha256.is_some())
    {
        return Err(ParentAuthError::InvalidGrant);
    }
    Ok(())
}

fn validate_transfer(
    transfer: &TransferGrantPayload,
    owner: &OwnerBindingPayload,
    worker: &WorkerBinding,
    role: WorkerRole,
    now: u64,
) -> Result<(), ParentAuthError> {
    let (asset_id, purpose) = match role {
        WorkerRole::Asr => ("whisper-base", "asr_weights"),
        WorkerRole::Llm => ("qwen3-4b-q4-k-m", "llm_weights"),
    };
    if transfer.schema != "saturday.voice.transfer-grant.v1"
        || transfer.request_role != "voice_asset_install"
        || transfer.generation == 0
        || transfer.grant_id_hex.len() != 32
        || !valid_hex(&transfer.grant_id_hex)
        || transfer.asset_id != asset_id
        || transfer.purpose != purpose
        || transfer.worker_role != role
        || transfer.expected_bytes == 0
        || !valid_sha256(&transfer.expected_sha256)
        || !valid_sha256(&transfer.binding_sha256)
        || transfer.binding_sha256 != owner.binding_sha256
        || transfer.source_generation_sha256 != worker.source_generation_sha256
        || transfer.source_tree_sha256 != worker.source_tree_sha256
        || transfer.resource_generation_sha256 != worker.resource_generation_sha256
        || transfer.worker_sha256 != worker.worker_sha256
        || transfer.worker_bytes != worker.worker_bytes
        || transfer.worker_source_seal_sha256 != worker.source_seal_sha256
        || transfer.worker_resource_seal_sha256 != worker.resource_seal_sha256
        || transfer.application_bundle_identifier != owner.application_bundle_identifier
        || transfer.application_team_id_sha256 != owner.application_team_id_sha256
        || transfer.application_requirement_sha256 != owner.application_requirement_sha256
        || transfer.parent_executable_sha256 != owner.parent_executable_sha256
        || transfer.parent_resource_closure_sha256 != owner.parent_resource_closure_sha256
        || transfer.worker_team_id_sha256 != worker.worker_team_id_sha256
        || transfer.worker_requirement_sha256 != worker.worker_requirement_sha256
        || transfer.issued_at_unix == 0
        || transfer.expires_at_unix <= transfer.issued_at_unix
        || transfer.expires_at_unix - transfer.issued_at_unix > 900
        || transfer.use_expires_at_unix <= transfer.issued_at_unix
        || transfer.use_expires_at_unix - transfer.issued_at_unix > MAX_PARENT_AUTH_TTL_SECONDS
        || now > transfer.use_expires_at_unix
        || !valid_sha256(&transfer.consent_nonce_sha256)
    {
        return Err(ParentAuthError::InvalidGrant);
    }
    Ok(())
}

fn validate_model_purpose(
    purpose: &ModelPurposePayload,
    transfer: &TransferGrantPayload,
    owner: &OwnerBindingPayload,
    worker: &WorkerBinding,
    now: u64,
) -> Result<(), ParentAuthError> {
    if purpose.schema != "saturday.voice.model-purpose.v1"
        || purpose.serial <= transfer.generation
        || purpose.asset_id != transfer.asset_id
        || purpose.model_sha256 != transfer.expected_sha256
        || !valid_sha256(&purpose.model_sha256)
        || !valid_sha256(&purpose.template_sha256)
        || !valid_sha256(&purpose.tokenizer_metadata_sha256)
        || purpose.architecture != "qwen3"
        || purpose.context_length != 32_768
        || purpose.binding_sha256 != owner.binding_sha256
        || purpose.source_generation_sha256 != worker.source_generation_sha256
        || purpose.source_tree_sha256 != worker.source_tree_sha256
        || purpose.resource_generation_sha256 != worker.resource_generation_sha256
        || purpose.worker_sha256 != worker.worker_sha256
        || purpose.worker_source_seal_sha256 != worker.source_seal_sha256
        || purpose.worker_resource_seal_sha256 != worker.resource_seal_sha256
        || purpose.application_bundle_identifier != owner.application_bundle_identifier
        || purpose.application_team_id_sha256 != owner.application_team_id_sha256
        || purpose.application_requirement_sha256 != owner.application_requirement_sha256
        || purpose.parent_executable_sha256 != owner.parent_executable_sha256
        || purpose.parent_resource_closure_sha256 != owner.parent_resource_closure_sha256
        || purpose.worker_team_id_sha256 != worker.worker_team_id_sha256
        || purpose.worker_requirement_sha256 != worker.worker_requirement_sha256
        || purpose.issued_at_unix < transfer.issued_at_unix
        || purpose.expires_at_unix <= purpose.issued_at_unix
        || purpose.expires_at_unix - purpose.issued_at_unix > MAX_PARENT_AUTH_TTL_SECONDS
        || now > purpose.expires_at_unix
        || !valid_sha256(&purpose.consent_nonce_sha256)
    {
        return Err(ParentAuthError::InvalidGrant);
    }
    Ok(())
}

fn verify_live_parent(pid: libc::pid_t, owner: &OwnerBindingPayload) -> Result<(), ParentAuthError> {
    let identity = live_identity(pid)?;
    if sha256_hex(&identity.team) != owner.application_team_id_sha256
        || std::str::from_utf8(&identity.identifier).ok() != Some(owner.application_bundle_identifier.as_str())
        || sha256_hex(&identity.requirement) != owner.application_requirement_sha256
    {
        return Err(ParentAuthError::ParentIdentityMismatch);
    }
    let path = pid_path(pid)?;
    if hash_file(&path)? != owner.parent_executable_sha256 {
        return Err(ParentAuthError::ParentIdentityMismatch);
    }
    let bundle_resources = path.parent().and_then(Path::parent)
        .map(|contents| contents.join("Resources"))
        .ok_or(ParentAuthError::ParentIdentityMismatch)?;
    let resource_dir = bundle_resources.join("saturday-resources");
    let binding_path = resource_dir.join("voice-asset-bindings.json");
    let binding_bytes = read_bounded(&binding_path, 256 * 1024)?;
    if sha256_hex(&binding_bytes) != owner.binding_sha256 {
        return Err(ParentAuthError::ResourceClosureMismatch);
    }
    let manifest_path = resource_dir.join("voice-runtime-manifest.json");
    let manifest_bytes = read_bounded(&manifest_path, MAX_RUNTIME_MANIFEST_BYTES)?;
    if sha256_hex(&manifest_bytes) != owner.resource_manifest_sha256
        || runtime_resource_closure_sha256(&owner.binding_sha256, &manifest_bytes)?
            != owner.parent_resource_closure_sha256
    {
        return Err(ParentAuthError::ResourceClosureMismatch);
    }
    let manifest: RuntimeManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
    if canonical_manifest(&manifest)? != manifest_bytes {
        return Err(ParentAuthError::ResourceClosureMismatch);
    }
    verify_runtime_manifest(&manifest, &resource_dir, owner)?;
    Ok(())
}

fn verify_runtime_resources(owner: &OwnerBindingPayload) -> Result<(), ParentAuthError> {
    // The live-parent verifier has already read and verified all manifest-listed resource bytes.
    // This second pass only validates owner-binding fields that can be checked without a path.
    if owner.resource_manifest_sha256 != owner.workers[0].resource_generation_sha256
        || owner.resource_manifest_sha256 != owner.workers[1].resource_generation_sha256
    {
        return Err(ParentAuthError::ResourceClosureMismatch);
    }
    Ok(())
}

fn verify_runtime_manifest(
    manifest: &RuntimeManifest,
    resource_dir: &Path,
    owner: &OwnerBindingPayload,
) -> Result<(), ParentAuthError> {
    if manifest.schema != "saturday.voice.runtime-resource-closure.v1"
        || manifest.workers.len() != MAX_RUNTIME_WORKERS
        || manifest.source_generations.len() != MAX_RUNTIME_WORKERS
        || manifest.resources.len() > MAX_RUNTIME_RESOURCES
        || manifest.workers[0].role != WorkerRole::Asr
        || manifest.workers[1].role != WorkerRole::Llm
        || manifest.source_generations[0].role != WorkerRole::Asr
        || manifest.source_generations[1].role != WorkerRole::Llm
        || manifest.resources.windows(2).any(|pair| pair[0].path >= pair[1].path)
    {
        return Err(ParentAuthError::ResourceClosureMismatch);
    }
    for worker in &owner.workers {
        let manifest_worker = manifest.workers.iter().find(|item| item.role == worker.role)
            .ok_or(ParentAuthError::ResourceClosureMismatch)?;
        if manifest_worker.asset_id != worker.asset_id
            || manifest_worker.bytes != worker.worker_bytes
            || manifest_worker.sha256 != worker.worker_sha256
            || manifest_worker.team_id_sha256 != worker.worker_team_id_sha256
            || manifest_worker.designated_requirement_sha256 != worker.worker_requirement_sha256
            || manifest_worker.source_seal_sha256 != worker.source_seal_sha256
            || manifest_worker.resource_seal_sha256 != worker.resource_seal_sha256
            || manifest_worker.max_rss_bytes != worker.max_rss_bytes
            || worker.resource_generation_sha256 != owner.resource_manifest_sha256
        {
            return Err(ParentAuthError::ResourceClosureMismatch);
        }
        let source = manifest.source_generations.iter().find(|item| item.role == worker.role)
            .ok_or(ParentAuthError::ResourceClosureMismatch)?;
        if source.source_tree_sha256 != worker.source_tree_sha256
            || source.source_revision != worker.source_revision
            || source.source_generation_sha256 != worker.source_generation_sha256
        {
            return Err(ParentAuthError::ResourceClosureMismatch);
        }
        let worker_path = safe_resource_path(resource_dir, &manifest_worker.path)?;
        if file_size(&worker_path)? != worker.worker_bytes || hash_file(&worker_path)? != worker.worker_sha256 {
            return Err(ParentAuthError::ResourceClosureMismatch);
        }
        let code_identity = live_identity_for_path(&worker_path)?;
        if sha256_hex(&code_identity.team) != worker.worker_team_id_sha256
            || sha256_hex(&code_identity.requirement) != worker.worker_requirement_sha256
        {
            return Err(ParentAuthError::ResourceClosureMismatch);
        }
    }
    for resource in &manifest.resources {
        if resource.bytes == 0 || !valid_sha256(&resource.sha256) || resource.mode > 0o777 {
            return Err(ParentAuthError::ResourceClosureMismatch);
        }
        let path = safe_resource_path(resource_dir, &resource.path)?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
        if !metadata.is_file() || metadata.file_type().is_symlink()
            || metadata.len() != resource.bytes
            || metadata.mode() & 0o777 != resource.mode
            || hash_file(&path)? != resource.sha256
        {
            return Err(ParentAuthError::ResourceClosureMismatch);
        }
    }
    let mut expected_files = std::collections::BTreeSet::new();
    expected_files.insert("voice-asset-bindings.json".to_owned());
    expected_files.insert("voice-runtime-manifest.json".to_owned());
    for worker in &manifest.workers {
        if !expected_files.insert(worker.path.clone()) {
            return Err(ParentAuthError::ResourceClosureMismatch);
        }
    }
    for resource in &manifest.resources {
        if !expected_files.insert(resource.path.clone()) {
            return Err(ParentAuthError::ResourceClosureMismatch);
        }
    }
    let (actual_files, actual_directories) = inventory_runtime_tree(resource_dir)?;
    if actual_files != expected_files || actual_directories != expected_parent_directories(&expected_files)? {
        return Err(ParentAuthError::ResourceClosureMismatch);
    }
    Ok(())
}

fn expected_parent_directories(
    files: &std::collections::BTreeSet<String>,
) -> Result<std::collections::BTreeSet<String>, ParentAuthError> {
    let mut directories = std::collections::BTreeSet::new();
    for file in files {
        let mut components = Path::new(file).components().peekable();
        let mut prefix = PathBuf::new();
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err(ParentAuthError::ResourceClosureMismatch);
            };
            prefix.push(name);
            if components.peek().is_some() {
                directories.insert(prefix.to_str().ok_or(ParentAuthError::ResourceClosureMismatch)?.to_owned());
            }
        }
    }
    Ok(directories)
}

fn inventory_runtime_tree(
    root: &Path,
) -> Result<(std::collections::BTreeSet<String>, std::collections::BTreeSet<String>), ParentAuthError> {
    let root_metadata = std::fs::symlink_metadata(root).map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(ParentAuthError::ResourceClosureMismatch);
    }
    let mut files = std::collections::BTreeSet::new();
    let mut directories = std::collections::BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|_| ParentAuthError::ResourceClosureMismatch)? {
            let entry = entry.map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
            let relative = path.strip_prefix(root).map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
            let relative = relative.to_str().ok_or(ParentAuthError::ResourceClosureMismatch)?;
            if relative.len() > MAX_PATH_BYTES
                || relative.is_empty()
                || relative.split('/').any(|component| component.is_empty() || component == "." || component == "..")
                || metadata.file_type().is_symlink()
            {
                return Err(ParentAuthError::ResourceClosureMismatch);
            }
            if metadata.is_dir() {
                directories.insert(relative.to_owned());
                pending.push(path);
            } else if metadata.is_file() {
                files.insert(relative.to_owned());
            } else {
                return Err(ParentAuthError::ResourceClosureMismatch);
            }
        }
    }
    Ok((files, directories))
}

fn verify_live_worker(role: WorkerRole, binding: &WorkerBinding, expected_sha: &str) -> Result<(), ParentAuthError> {
    let pid = unsafe { libc::getpid() };
    let identity = live_identity(pid)?;
    if sha256_hex(&identity.team) != binding.worker_team_id_sha256
        || sha256_hex(&identity.requirement) != binding.worker_requirement_sha256
        || binding.role != role
        || binding.worker_sha256 != expected_sha
        || hash_file(&pid_path(pid)?)? != expected_sha
    {
        return Err(ParentAuthError::WorkerIdentityMismatch);
    }
    Ok(())
}

fn live_identity(pid: libc::pid_t) -> Result<LiveCodeIdentity, ParentAuthError> {
    #[cfg(target_os = "macos")]
    {
        let mut team = std::ptr::null();
        let mut identifier = std::ptr::null();
        let mut requirement = std::ptr::null();
        let status = unsafe { saturday_parent_auth_copy_live_identity(pid, &mut team, &mut identifier, &mut requirement) };
        if status != 0 {
            release_cf_data(team);
            release_cf_data(identifier);
            release_cf_data(requirement);
            return Err(ParentAuthError::ParentIdentityMismatch);
        }
        let team = copy_cf_data(team);
        let team = match team {
            Ok(value) => value,
            Err(error) => {
                release_cf_data(identifier);
                release_cf_data(requirement);
                return Err(error);
            }
        };
        let identifier = match copy_cf_data(identifier) {
            Ok(value) => value,
            Err(error) => {
                release_cf_data(requirement);
                return Err(error);
            }
        };
        let requirement = copy_cf_data(requirement)?;
        Ok(LiveCodeIdentity { team, identifier, requirement })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        Err(ParentAuthError::PlatformUnavailable)
    }
}

fn live_identity_for_path(path: &Path) -> Result<LiveCodeIdentity, ParentAuthError> {
    #[cfg(target_os = "macos")]
    {
        let path = std::ffi::CString::new(path.to_str().ok_or(ParentAuthError::ResourceClosureMismatch)?)
            .map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
        let mut team = std::ptr::null();
        let mut identifier = std::ptr::null();
        let mut requirement = std::ptr::null();
        let status = unsafe {
            saturday_parent_auth_copy_static_identity(path.as_ptr(), &mut team, &mut identifier, &mut requirement)
        };
        if status != 0 {
            release_cf_data(team);
            release_cf_data(identifier);
            release_cf_data(requirement);
            return Err(ParentAuthError::ResourceClosureMismatch);
        }
        let team = copy_cf_data(team);
        let team = match team {
            Ok(value) => value,
            Err(error) => {
                release_cf_data(identifier);
                release_cf_data(requirement);
                return Err(error);
            }
        };
        let identifier = match copy_cf_data(identifier) {
            Ok(value) => value,
            Err(error) => {
                release_cf_data(requirement);
                return Err(error);
            }
        };
        let requirement = copy_cf_data(requirement)?;
        Ok(LiveCodeIdentity { team, identifier, requirement })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err(ParentAuthError::PlatformUnavailable)
    }
}

#[derive(Debug, Serialize)]
pub struct SignedCodeIdentity {
    pub team_identifier: String,
    pub identifier: String,
    pub designated_requirement_sha256: String,
}

pub fn inspect_signed_code_at(path: &Path) -> Result<SignedCodeIdentity, ParentAuthError> {
    let identity = live_identity_for_path(path)?;
    if identity.team.is_empty() || identity.identifier.is_empty() || identity.requirement.is_empty() {
        return Err(ParentAuthError::WorkerIdentityMismatch);
    }
    Ok(SignedCodeIdentity {
        team_identifier: String::from_utf8(identity.team).map_err(|_| ParentAuthError::WorkerIdentityMismatch)?,
        identifier: String::from_utf8(identity.identifier).map_err(|_| ParentAuthError::WorkerIdentityMismatch)?,
        designated_requirement_sha256: sha256_hex(&identity.requirement),
    })
}


fn verify_signed<T: Serialize>(domain: &[u8], signed: &Signed<T>, public_key: &[u8]) -> Result<bool, ParentAuthError> {
    verify_signed_payload(domain, &signed.payload, &signed.signature_der, public_key)
}

fn verify_signed_payload<T: Serialize>(
    domain: &[u8],
    payload: &T,
    signature: &[u8],
    public_key: &[u8],
) -> Result<bool, ParentAuthError> {
    if signature.len() < 8 || signature.len() > MAX_SIGNATURE_BYTES || signature.first() != Some(&0x30) {
        return Err(ParentAuthError::InvalidSignature);
    }
    let payload = serde_json::to_vec(payload).map_err(|_| ParentAuthError::InvalidGrant)?;
    if payload.is_empty() || payload.len() > MAX_PARENT_GRANT_BYTES {
        return Err(ParentAuthError::InvalidGrant);
    }
    let mut message = Vec::with_capacity(domain.len() + payload.len());
    message.extend_from_slice(domain);
    message.extend_from_slice(&payload);
    verify_signature(public_key, &message, signature)
}

fn verify_signature(public_key: &[u8], message: &[u8], signature: &[u8]) -> Result<bool, ParentAuthError> {
    #[cfg(target_os = "macos")]
    {
        let status = unsafe { saturday_parent_auth_verify_signature(
            public_key.as_ptr(), public_key.len(), message.as_ptr(), message.len(),
            signature.as_ptr(), signature.len(),
        ) };
        match status { 0 => Ok(true), -3 => Ok(false), _ => Err(ParentAuthError::InvalidSignature) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (public_key, message, signature);
        Err(ParentAuthError::PlatformUnavailable)
    }
}

pub fn canonical_signed_payload<T: Serialize>(domain: &[u8], payload: &T) -> Result<Vec<u8>, ParentAuthError> {
    let body = serde_json::to_vec(payload).map_err(|_| ParentAuthError::InvalidGrant)?;
    if body.is_empty() || body.len() + domain.len() > MAX_PARENT_GRANT_BYTES {
        return Err(ParentAuthError::InvalidGrant);
    }
    let mut result = Vec::with_capacity(domain.len() + body.len());
    result.extend_from_slice(domain);
    result.extend_from_slice(&body);
    Ok(result)
}

pub fn runtime_resource_closure_sha256(binding_sha256: &str, manifest_bytes: &[u8]) -> Result<String, ParentAuthError> {
    if !valid_sha256(binding_sha256) || manifest_bytes.is_empty() || manifest_bytes.len() > MAX_RUNTIME_MANIFEST_BYTES {
        return Err(ParentAuthError::ResourceClosureMismatch);
    }
    let mut digest = Sha256::new();
    digest.update(RESOURCE_CLOSURE_DOMAIN);
    digest.update(binding_sha256.as_bytes());
    digest.update([0]);
    digest.update(Sha256::digest(manifest_bytes));
    Ok(format!("{:x}", digest.finalize()))
}

fn validate_owner_binding(owner: &OwnerBindingPayload) -> Result<(), ParentAuthError> {
    owner.validate()
}

fn key_id_sha256() -> String { sha256_hex(OWNER_KEY_APPLICATION_TAG) }

fn now_unix_seconds() -> Result<u64, ParentAuthError> {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|value| value.as_secs())
        .map_err(|_| ParentAuthError::Expired)
}


fn current_process_unique_id() -> Result<u64, ParentAuthError> {
    #[cfg(target_os = "macos")]
    {
        let mut unique_id = 0u64;
        let status = unsafe {
            saturday_parent_auth_copy_pid_unique_id(libc::getpid(), &mut unique_id)
        };
        if status != 0 || unique_id == 0 {
            return Err(ParentAuthError::ParentIdentityMismatch);
        }
        Ok(unique_id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(ParentAuthError::PlatformUnavailable)
    }
}
fn pid_path(pid: libc::pid_t) -> Result<PathBuf, ParentAuthError> {
    #[cfg(target_os = "macos")]
    {
        let mut buffer = [0u8; MAX_PATH_BYTES];
        let status = unsafe { saturday_parent_auth_copy_pid_path(pid, buffer.as_mut_ptr().cast(), buffer.len()) };
        if status != 0 { return Err(ParentAuthError::ParentIdentityMismatch); }
        let nul = buffer.iter().position(|value| *value == 0).ok_or(ParentAuthError::ParentIdentityMismatch)?;
        let path = std::str::from_utf8(&buffer[..nul]).map_err(|_| ParentAuthError::ParentIdentityMismatch)?;
        Ok(PathBuf::from(path))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        Err(ParentAuthError::PlatformUnavailable)
    }
}

fn safe_resource_path(root: &Path, value: &str) -> Result<PathBuf, ParentAuthError> {
    if value.is_empty() || value.len() > MAX_PATH_BYTES || value.contains('\0') {
        return Err(ParentAuthError::ResourceClosureMismatch);
    }
    let relative = Path::new(value);
    if relative.is_absolute() || relative.components().any(|component| !matches!(component, Component::Normal(_))) {
        return Err(ParentAuthError::ResourceClosureMismatch);
    }
    let mut path = root.to_owned();
    for component in relative.components() {
        path.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
        if metadata.file_type().is_symlink() { return Err(ParentAuthError::ResourceClosureMismatch); }
    }
    Ok(path)
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, ParentAuthError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 || metadata.len() > maximum as u64 {
        return Err(ParentAuthError::ResourceClosureMismatch);
    }
    let file = File::open(path).map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum as u64 + 1).read_to_end(&mut bytes).map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
    if bytes.len() as u64 != metadata.len() { return Err(ParentAuthError::ResourceClosureMismatch); }
    Ok(bytes)
}

fn hash_file(path: &Path) -> Result<String, ParentAuthError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| ParentAuthError::ParentIdentityMismatch)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() { return Err(ParentAuthError::ParentIdentityMismatch); }
    let mut file = File::open(path).map_err(|_| ParentAuthError::ParentIdentityMismatch)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| ParentAuthError::ParentIdentityMismatch)?;
        if count == 0 { break; }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn file_size(path: &Path) -> Result<u64, ParentAuthError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| ParentAuthError::ResourceClosureMismatch)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() { return Err(ParentAuthError::ResourceClosureMismatch); }
    Ok(metadata.len())
}

fn sha256_hex(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn valid_sha256(value: &str) -> bool { value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) }
fn valid_hex(value: &str) -> bool { !value.is_empty() && value.len().is_multiple_of(2) && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) }
fn valid_bundle_id(value: &str) -> bool { !value.is_empty() && value.len() <= 256 && !value.contains('\0') && value.split('.').all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')) }

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ParentAuthError> {
    let value = serde_json::to_value(value).map_err(|_| ParentAuthError::InvalidGrant)?;
    serde_json::to_vec(&value).map_err(|_| ParentAuthError::InvalidGrant)
}

fn canonical_manifest(value: &RuntimeManifest) -> Result<Vec<u8>, ParentAuthError> {
    let mut bytes = canonical_json(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn copy_cf_data(value: saturday_parent_auth_cf_data_ref) -> Result<Vec<u8>, ParentAuthError> {
    #[cfg(target_os = "macos")]
    {
        if value.is_null() { return Err(ParentAuthError::ParentIdentityMismatch); }
        let length = unsafe { CFDataGetLength(value) };
        let bytes = unsafe { CFDataGetBytePtr(value) };
        if length <= 0 || length > 4096 || bytes.is_null() { release_cf_data(value); return Err(ParentAuthError::ParentIdentityMismatch); }
        let result = unsafe { std::slice::from_raw_parts(bytes, length as usize) }.to_vec();
        release_cf_data(value);
        Ok(result)
    }
    #[cfg(not(target_os = "macos"))]
    { let _ = value; Err(ParentAuthError::PlatformUnavailable) }
}

fn release_cf_data(value: saturday_parent_auth_cf_data_ref) {
    #[cfg(target_os = "macos")]
    if !value.is_null() { unsafe { saturday_parent_auth_cf_release(value) }; }
    #[cfg(not(target_os = "macos"))]
    let _ = value;
}

#[allow(non_camel_case_types)]
type saturday_parent_auth_cf_data_ref = *const std::ffi::c_void;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn saturday_parent_auth_verify_signature(public_bytes: *const u8, public_len: usize,
        message_bytes: *const u8, message_len: usize, signature_bytes: *const u8, signature_len: usize) -> i32;
    fn saturday_parent_auth_copy_live_identity(pid: i32, team_out: *mut saturday_parent_auth_cf_data_ref,
        identifier_out: *mut saturday_parent_auth_cf_data_ref, requirement_out: *mut saturday_parent_auth_cf_data_ref) -> i32;
    fn saturday_parent_auth_copy_static_identity(path_utf8: *const std::ffi::c_char,
        team_out: *mut saturday_parent_auth_cf_data_ref, identifier_out: *mut saturday_parent_auth_cf_data_ref,

        requirement_out: *mut saturday_parent_auth_cf_data_ref) -> i32;
    fn saturday_parent_auth_copy_pid_path(pid: i32, output: *mut std::ffi::c_char, capacity: usize) -> i32;
    fn saturday_parent_auth_copy_pid_unique_id(pid: i32, unique_id_out: *mut u64) -> i32;
    fn saturday_parent_auth_cf_release(value: saturday_parent_auth_cf_data_ref);
    fn CFDataGetLength(data: saturday_parent_auth_cf_data_ref) -> isize;
    fn CFDataGetBytePtr(data: saturday_parent_auth_cf_data_ref) -> *const u8;
}

#[cfg(test)]
mod bootstrap_record_tests {
    use super::*;

    #[test]
    fn bootstrap_record_rejects_empty_oversized_and_duplicate_members() {
        assert_eq!(verify_owner_key_bootstrap_record(&[]), Err(ParentAuthError::InvalidGrant));
        assert_eq!(
            verify_owner_key_bootstrap_record(&vec![b' '; MAX_BOOTSTRAP_RECORD_BYTES + 1]),
            Err(ParentAuthError::InvalidGrant),
        );
        let duplicate = br#"{"payload":{},"payload":{},"signature_der":[]}"#;
        assert_eq!(verify_owner_key_bootstrap_record(duplicate), Err(ParentAuthError::InvalidGrant));
    }

    #[test]
    fn bootstrap_record_rejects_invalid_key_and_signature_bounds_before_crypto() {
        let payload = OwnerKeyBootstrapPayload {
            schema: OWNER_KEY_BOOTSTRAP_SCHEMA.to_owned(),
            application_tag_sha256: sha256_hex(OWNER_KEY_APPLICATION_TAG),
            owner_public_key_x963: vec![0x04; 65],
            owner_public_key_sha256: "0".repeat(64),
            application_bundle_identifier: "dev.example.app".to_owned(),
            application_team_id_sha256: "1".repeat(64),
            application_requirement_sha256: "2".repeat(64),
            issued_at_unix: 1,
            nonce_sha256: "3".repeat(64),
        };
        let signed = Signed { payload, signature_der: vec![0x30; MAX_SIGNATURE_BYTES + 1] };
        let bytes = serde_json::to_vec(&signed).unwrap();
        assert_eq!(verify_owner_key_bootstrap_record(&bytes), Err(ParentAuthError::InvalidGrant));
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_truncated_and_oversized_parent_grants() {
        let trusted_key = [0x04; 65];
        let context = ParentWorkerGrantContext {
            observed_parent_pid: 42,
            role: WorkerRole::Asr,
            worker_sha256: &"a".repeat(64),
            source_seal_sha256: &"b".repeat(64),
            resource_seal_sha256: &"c".repeat(64),
            trusted_owner_public_key_x963: &trusted_key,
            expected_challenge_hex: &"0".repeat(64),
        };
        assert_eq!(verify_parent_worker_grant(&[], context), Err(ParentAuthError::InvalidGrant));
        assert_eq!(verify_parent_worker_grant(&vec![b' '; MAX_PARENT_GRANT_BYTES + 1], context), Err(ParentAuthError::InvalidGrant));
    }

    #[test]
    fn resource_closure_changes_when_manifest_bytes_change() {
        let binding = "a".repeat(64);
        let first = runtime_resource_closure_sha256(&binding, br#"{"schema":"saturday.voice.runtime-resource-closure.v1"}"#).unwrap();
        let second = runtime_resource_closure_sha256(&binding, br#"{"schema":"saturday.voice.runtime-resource-closure.v1","changed":true}"#).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn rejects_noncanonical_and_duplicate_parent_grant_members_before_keychain() {
        let duplicate = br#"{"schema":"a","schema":"b","worker_role":"asr","owner_binding":{},"asset_grant":{},"model_purpose":null,"authorization_state":{}}"#;
        assert_eq!(verify_parent_worker_grant(duplicate, ParentWorkerGrantContext {
            observed_parent_pid: 42,
            role: WorkerRole::Asr,
            worker_sha256: &"a".repeat(64),
            source_seal_sha256: &"b".repeat(64),
            resource_seal_sha256: &"c".repeat(64),
            trusted_owner_public_key_x963: &[0x04; 65],
            expected_challenge_hex: &"0".repeat(64),
        }), Err(ParentAuthError::InvalidGrant));
    }

    #[test]
    fn rejects_wrong_worker_role_before_owner_key_access() {
        let empty_hash = "a".repeat(64);
        let grant = serde_json::json!({
            "schema": PARENT_GRANT_SCHEMA,
            "worker_role": "asr",
            "owner_binding": {
                "payload": {
                    "schema": "saturday.voice.owner-binding.v1",
                    "binding_sha256": empty_hash,
                    "key_id_sha256": "b".repeat(64),
                    "owner_public_key_sha256": "c".repeat(64),
                    "reviewer_roster_sha256": "d".repeat(64),
                    "owner_public_key_x963": vec![4u8; 65],
                    "generation": 1,
                    "application_bundle_identifier": "dev.example.app",
                    "application_team_id_sha256": "e".repeat(64),
                    "application_requirement_sha256": "f".repeat(64),
                    "parent_executable_sha256": "1".repeat(64),
                    "parent_resource_closure_sha256": "2".repeat(64),
                    "resource_manifest_sha256": "3".repeat(64),
                    "workers": [],
                    "issued_at_unix": 1,
                    "expires_at_unix": 2,
                    "nonce_sha256": "4".repeat(64)
                },
                "signature_der": []
            },
            "asset_grant": {
                "payload": {
                    "schema": "saturday.voice.transfer-grant.v1",
                    "request_role": "voice_asset_install",
                    "generation": 1,
                    "grant_id_hex": "11111111111111111111111111111111",
                    "asset_id": "whisper-base",
                    "purpose": "asr_weights",
                    "worker_role": "asr",
                    "expected_bytes": 1,
                    "expected_sha256": "a".repeat(64),
                    "binding_sha256": "b".repeat(64),
                    "source_generation_sha256": "c".repeat(64),
                    "source_tree_sha256": "d".repeat(64),
                    "resource_generation_sha256": "e".repeat(64),
                    "worker_sha256": "f".repeat(64),
                    "worker_bytes": 1,
                    "worker_source_seal_sha256": "1".repeat(64),
                    "worker_resource_seal_sha256": "2".repeat(64),
                    "template_fingerprint_sha256": null,
                    "application_bundle_identifier": "dev.example.app",
                    "application_team_id_sha256": "3".repeat(64),
                    "application_requirement_sha256": "4".repeat(64),
                    "parent_executable_sha256": "5".repeat(64),
                    "parent_resource_closure_sha256": "6".repeat(64),
                    "worker_team_id_sha256": "7".repeat(64),
                    "worker_requirement_sha256": "8".repeat(64),
                    "issued_at_unix": 1,
                    "expires_at_unix": 2,
                    "use_expires_at_unix": 3,
                    "consent_nonce_sha256": "9".repeat(64)
                },
                "signature_der": []
            },
            "model_purpose": null,
            "authorization_state": {
                "payload": {
                    "schema": AUTHORIZATION_STATE_SCHEMA,
                    "binding_sha256": "b".repeat(64),
                    "serial": 1,
                    "ledger_sha256": "c".repeat(64),
                    "active_assets": []
                },
                "signature_der": []
            },
            "launch_authorization": {
                "payload": {
                    "schema": "saturday.voice.worker-launch-authorization.v1",
                    "challenge_hex": "a".repeat(64),
                    "child_pid": 42,
                    "child_unique_id": 1,
                    "worker_role": "asr",
                    "asset_id": "whisper-base",
                    "generation": 1,
                    "binding_sha256": "b".repeat(64),
                    "owner_public_key_sha256": "c".repeat(64),
                    "parent_executable_sha256": "5".repeat(64),
                    "parent_resource_closure_sha256": "6".repeat(64),
                    "authorization_state_serial": 1,
                    "issued_at_unix": 1,
                    "expires_at_unix": 2,
                    "nonce_sha256": "d".repeat(64)
                },
                "signature_der": []
            }
        });
        let bytes = canonical_json(&grant).unwrap();
        assert_eq!(
            verify_parent_worker_grant(&bytes, ParentWorkerGrantContext {
                observed_parent_pid: 42,
                role: WorkerRole::Llm,
                worker_sha256: &"a".repeat(64),
                source_seal_sha256: &"b".repeat(64),
                resource_seal_sha256: &"c".repeat(64),
                trusted_owner_public_key_x963: &[0x04; 65],
                expected_challenge_hex: &"a".repeat(64),
            }),
            Err(ParentAuthError::InvalidGrant),
        );
    }

    #[test]
    fn rejects_oversized_signature_before_crypto_dispatch() {
        let signed = Signed {
            payload: serde_json::json!({"record": "test"}),
            signature_der: vec![0x30; MAX_SIGNATURE_BYTES + 1],
        };
        assert_eq!(
            verify_signed(b"test-domain", &signed, &[4; 65]),
            Err(ParentAuthError::InvalidSignature),
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_worker_signature_verifier_accepts_known_p256_vector_and_rejects_mutation() {
        fn from_hex(value: &str) -> Vec<u8> {
            value.as_bytes().chunks_exact(2).map(|pair| {
                let byte = std::str::from_utf8(pair).unwrap();
                u8::from_str_radix(byte, 16).unwrap()
            }).collect()
        }

        let mut public_key = vec![0x04];
        public_key.extend(from_hex(
            "60fed4ba255a9d31c961eb74c6356d68c049b8923b61fa6ce669622e60f29fb6\
             7903fe1008b8bc99a41ae9e95628bc64f2f1b20c2d7e9f5177a3c294d4462299",
        ));
        let signature = from_hex(
            "3046022100efd48b2aacb6a8fd1140dd9cd45e81d69d2c877b56aaf991c34d0ea84eaf3716\
             022100f7cb1c942d657c41d436c7a1b6e29f65f3e900dbb9aff4064dc4ab2f843acda8",
        );
        assert_eq!(verify_signature(&public_key, b"sample", &signature), Ok(true));
        let mut mutated = signature;
        *mutated.last_mut().unwrap() ^= 1;
        assert_eq!(verify_signature(&public_key, b"sample", &mutated), Ok(false));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn process_incarnation_token_is_stable_for_current_pid() {
        let first = current_process_unique_id().expect("current process identity is available");
        let second = current_process_unique_id().expect("current process identity remains available");

        assert_ne!(first, 0);
        assert_eq!(first, second);
    }

    #[test]
    fn disabled_or_replaced_receipt_rejects_a_previously_issued_worker_grant() {
        let sha256 = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";
        let receipt = AuthorizationAssetState {
            asset_id: "whisper-base".to_owned(),
            purpose: "asr_weights".to_owned(),
            generation: 9,
            version: sha256.to_owned(),
            bytes: 147_951_465,
            sha256: sha256.to_owned(),
            state: "enabled".to_owned(),
            template_fingerprint_sha256: None,
            tokenizer_metadata_sha256: None,
        };
        let expected = AssetGrantExpectation {
            asset_id: "whisper-base",
            purpose: "asr_weights",
            generation: 9,
            expected_bytes: 147_951_465,
            expected_sha256: sha256,
            role: WorkerRole::Asr,
            model_purpose: None,
        };
        assert!(asset_state_matches_grant(&receipt, expected));

        let mut disabled = receipt.clone();
        disabled.state = "disabled".to_owned();
        assert!(!asset_state_matches_grant(&disabled, expected));
        assert!(!asset_state_matches_grant(&receipt, AssetGrantExpectation {
            generation: 8,
            ..expected
        }));
    }
}
