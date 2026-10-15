use super::catalog::{
    is_unprovisioned_bootstrap_profile, load_embedded_catalog, FrozenAsset, FrozenCatalog,
};
use super::consent::{model_purpose_bytes, NativeVoiceConsentPrompt};
use super::ledger::{
    decode_lower_hex, encode_lower_hex, ActiveAssetReceipt, DurableLedger, GrantPayload,
    GrantRecord, GrantState, LockedLedger, PurposePayload, PurposeRecord,
};
use super::{
    current_unix_seconds, is_sha256, sha256_hex, ApprovedModelHandle, ApprovedRuntimeClosure,
    AssetPurpose, AuthorizedAssetState, OwnerConsent, OwnerConsentScope, RuntimeClosureBinding,
    TransferOutcome, TransferResult, VerifiedModelFile, VoiceAssetAuthority,
    VoiceAuthorizationError, VoiceBindingReview, VoiceGrant, VoiceKeyProof, VoiceKeySession,
    VoiceModelIdentity, VoiceOwnerBootstrapResult, VoiceOwnerBootstrapReview, VoiceOwnerKey,
    VoiceTransferLease, WorkerAuthorizationNeeds, WorkerRole, VOICE_GRANT_DOMAIN,
    VOICE_GRANT_SCHEMA, VOICE_KEY_APPLICATION_TAG, VOICE_MODEL_PURPOSE_DOMAIN,
    VOICE_OWNER_BINDING_DOMAIN, VOICE_PARENT_GRANT_TTL_SECONDS, VOICE_TRANSFER_TTL_SECONDS,
};
use crate::voice::contract::{AssetDisclosure, CancelToken};
use crate::voice::ipc::NativeCallerContext;
use saturday_voice_parent_auth::{
    self as parent_auth, ModelPurposePayload as ParentPurposePayload, OwnerBindingPayload,
    OwnerKeyBootstrapPayload, ParentWorkerGrant, RuntimeManifest, Signed as ParentSigned,
    TransferGrantPayload as ParentTransferGrantPayload, WorkerBinding as ParentWorkerBinding,
    WorkerLaunchAuthorizationPayload, WorkerRole as ParentWorkerRole,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

const PURPOSE_SCHEMA: &str = "saturday.voice.model-purpose.v1";
const MAX_SIGNED_RECORD_BYTES: usize = 16_384;
const PROMPT_WORKER_SESSION: &str = "Authorize local voice processing with the approved models";
const PROMPT_BOOTSTRAP: &str = "Create the dedicated Saturday local voice authorization key";
const PROMPT_GRANT: &str = "Authorize this reviewed local voice model transfer";
const PROMPT_PURPOSE: &str = "Authorize use of the reviewed local Qwen model";

#[derive(Clone)]
pub(crate) struct VoiceAuthorizationService {
    app: tauri::AppHandle,
    app_data_dir: PathBuf,
    resource_dir: PathBuf,
    key: Arc<dyn VoiceOwnerKey>,
}

#[derive(Debug, Clone)]
struct SessionAsset {
    role: WorkerRole,
    asset_id: String,
    sha256: String,
    generation: u64,
}

pub(crate) struct AuthorizedWorkerSession {
    service: Arc<VoiceAuthorizationService>,
    key_session: Arc<dyn VoiceKeySession>,
    valid: AtomicBool,
    gate: Mutex<()>,
    expires_at: Instant,
    state_serial: u64,
    binding_sha256: String,
    parent_executable_sha256: String,
    parent_resource_closure_sha256: String,
    owner_public_key_sha256: String,
    assets: Vec<SessionAsset>,
}

impl AuthorizedWorkerSession {
    pub(crate) fn is_usable(
        &self,
        required_until: Instant,
        required_asr: bool,
        required_llm: bool,
    ) -> bool {
        self.valid.load(Ordering::Acquire)
            && self.key_session.is_valid()
            && Instant::now() < self.expires_at
            && required_until <= self.expires_at
            && (!required_asr
                || self
                    .assets
                    .iter()
                    .any(|asset| asset.role == WorkerRole::Asr))
            && (!required_llm
                || self
                    .assets
                    .iter()
                    .any(|asset| asset.role == WorkerRole::Llm))
            && self
                .service
                .session_snapshot_is_current(self)
                .unwrap_or(false)
    }

    pub(crate) fn issue_parent_grant(
        &self,
        template: &ParentWorkerGrant,
        challenge_hex: &str,
        child_pid: u32,
        child_unique_id: u64,
        cancellation: &CancelToken,
    ) -> Result<Vec<u8>, VoiceAuthorizationError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
        if cancellation.is_cancelled()
            || child_pid == 0
            || child_unique_id == 0
            || challenge_hex.len() != 64
            || !challenge_hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || !self.valid.load(Ordering::Acquire)
            || !self.key_session.is_valid()
            || !self.service.session_snapshot_is_current(self)?
        {
            return Err(VoiceAuthorizationError::OwnerAuthentication);
        }
        let role = match template.worker_role {
            ParentWorkerRole::Asr => WorkerRole::Asr,
            ParentWorkerRole::Llm => WorkerRole::Llm,
        };
        let transfer = &template.asset_grant.payload;
        if template.schema != parent_auth::PARENT_GRANT_SCHEMA
            || template.launch_authorization.is_some()
            || transfer.worker_role != template.worker_role
            || transfer.binding_sha256 != self.binding_sha256
            || template.owner_binding.payload.owner_public_key_sha256
                != self.owner_public_key_sha256
            || template.owner_binding.payload.parent_executable_sha256
                != self.parent_executable_sha256
            || template
                .owner_binding
                .payload
                .parent_resource_closure_sha256
                != self.parent_resource_closure_sha256
            || template.authorization_state.payload.serial != self.state_serial
            || !self.assets.iter().any(|asset| {
                asset.role == role
                    && asset.asset_id == transfer.asset_id
                    && asset.sha256 == transfer.expected_sha256
                    && asset.generation == transfer.generation
            })
        {
            return Err(VoiceAuthorizationError::OwnerAuthentication);
        }
        let now = current_unix_seconds()?;
        let session_seconds = self
            .expires_at
            .saturating_duration_since(Instant::now())
            .as_secs();
        if session_seconds == 0 {
            return Err(VoiceAuthorizationError::OwnerAuthentication);
        }
        let expires_at_unix = now
            .checked_add(session_seconds.min(60))
            .ok_or(VoiceAuthorizationError::ProtocolFailure)?;
        let challenge = decode_lower_hex(challenge_hex)?;
        let payload = WorkerLaunchAuthorizationPayload {
            schema: "saturday.voice.worker-launch-authorization.v1".to_owned(),
            challenge_hex: challenge_hex.to_owned(),
            child_pid,
            child_unique_id,
            worker_role: template.worker_role,
            asset_id: transfer.asset_id.clone(),
            generation: transfer.generation,
            binding_sha256: self.binding_sha256.clone(),
            owner_public_key_sha256: self.owner_public_key_sha256.clone(),
            parent_executable_sha256: self.parent_executable_sha256.clone(),
            parent_resource_closure_sha256: self.parent_resource_closure_sha256.clone(),
            authorization_state_serial: self.state_serial,
            issued_at_unix: now,
            expires_at_unix,
            nonce_sha256: sha256_hex(&challenge),
        };
        let message = signed_bytes(parent_auth::WORKER_LAUNCH_AUTH_DOMAIN, &payload)?;
        let proof = self.key_session.sign(&message)?;
        if cancellation.is_cancelled()
            || Instant::now() >= self.expires_at
            || !self.key_session.is_valid()
            || !self.service.session_snapshot_is_current(self)?
            || sha256_hex(&proof.public_key_x963) != self.owner_public_key_sha256
            || !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(
                &proof.public_key_x963, &message, &proof.signature_der)?
        {
            return Err(VoiceAuthorizationError::OwnerAuthentication);
        }
        let mut grant = template.clone();
        grant.launch_authorization = Some(ParentSigned {
            payload,
            signature_der: proof.signature_der,
        });
        let bytes = canonical_parent_json(&grant)?;
        if bytes.is_empty() || bytes.len() > 8_192 {
            return Err(VoiceAuthorizationError::ProtocolFailure);
        }
        Ok(bytes)
    }

    pub(crate) fn invalidate(&self) {
        self.valid.store(false, Ordering::Release);
        self.key_session.invalidate();
    }
}

impl Drop for AuthorizedWorkerSession {
    fn drop(&mut self) {
        self.invalidate();
    }
}

fn validate_worker_session_request(
    now: Instant,
    deadline: Instant,
    needs: WorkerAuthorizationNeeds,
) -> Result<(), VoiceAuthorizationError> {
    if deadline <= now || deadline > now + Duration::from_secs(5 * 60) || (!needs.asr && !needs.llm)
    {
        return Err(VoiceAuthorizationError::ProtocolFailure);
    }
    Ok(())
}

fn session_state_serial_is_current(authorized: u64, current: u64) -> bool {
    authorized == current
}

impl VoiceAuthorizationService {
    pub(crate) fn from_app(app: tauri::AppHandle) -> Result<Self, VoiceAuthorizationError> {
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
        let resource_dir = app
            .path()
            .resource_dir()
            .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
        Ok(Self {
            app,
            app_data_dir,
            resource_dir,
            key: Arc::new(
                crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey::new(),
            ),
        })
    }

    fn catalog(&self) -> Result<FrozenCatalog, VoiceAuthorizationError> {
        verify_bundled_binding_resource(&self.resource_dir)?;
        load_embedded_catalog()
    }

    fn locked(&self, catalog: &FrozenCatalog) -> Result<LockedLedger, VoiceAuthorizationError> {
        let mut ledger = LockedLedger::open(&self.app_data_dir, &catalog.binding_sha256)?;
        match ledger.read_owner_anchor()? {
            Some(bytes) => {
                let anchor: ParentSigned<OwnerBindingPayload> = serde_json::from_slice(&bytes)
                    .map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
                if canonical_parent_json(&anchor)? != bytes {
                    return Err(VoiceAuthorizationError::StorageInvalid);
                }
                self.verify_owner_anchor(catalog, &mut ledger, &anchor)?;
                ledger.verify_snapshot(&anchor.payload.owner_public_key_x963)?;
            }
            None if ledger.value.serial == 0
                && ledger.value.pending.is_none()
                && ledger.value.history.is_empty()
                && ledger.value.active_assets.is_empty() => {}
            None => return Err(VoiceAuthorizationError::KeyUnavailable),
        }
        Ok(ledger)
    }

    fn signed_ledger_snapshot(
        &self,
        ledger: &mut LockedLedger,
    ) -> Result<parent_auth::SignedAuthorizationState, VoiceAuthorizationError> {
        ledger.value.serial = ledger
            .value
            .serial
            .checked_add(1)
            .ok_or(VoiceAuthorizationError::ProtocolFailure)?;
        let public_key = ledger
            .value
            .owner_public_key_x963
            .as_deref()
            .ok_or(VoiceAuthorizationError::KeyUnavailable)
            .and_then(decode_lower_hex)?;
        let payload = ledger.value.authorization_state_payload()?;
        let encoded =
            serde_json::to_vec(&payload).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
        if encoded.is_empty() || encoded.len() > parent_auth::MAX_AUTHORIZATION_STATE_BYTES {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        let mut message =
            Vec::with_capacity(parent_auth::AUTHORIZATION_STATE_DOMAIN.len() + encoded.len());
        message.extend_from_slice(parent_auth::AUTHORIZATION_STATE_DOMAIN);
        message.extend_from_slice(&encoded);
        let proof = self
            .key
            .sign(&message, "Seal local voice authorization state")?;
        if proof.public_key_x963 != public_key
            || !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(&public_key, &message, &proof.signature_der)?
        {
            return Err(VoiceAuthorizationError::OwnerAuthentication);
        }
        Ok(parent_auth::SignedAuthorizationState {
            payload,
            signature_der: proof.signature_der,
        })
    }

    fn persist_signed_ledger(
        &self,
        ledger: &mut LockedLedger,
    ) -> Result<(), VoiceAuthorizationError> {
        let authorization_state = self.signed_ledger_snapshot(ledger)?;
        ledger.persist(&authorization_state)
    }

    fn verify_owner_anchor(
        &self,
        catalog: &FrozenCatalog,
        ledger: &mut LockedLedger,
        anchor: &ParentSigned<OwnerBindingPayload>,
    ) -> Result<(), VoiceAuthorizationError> {
        let owner = &anchor.payload;
        owner
            .validate()
            .map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
        if anchor.signature_der.len() < 8
            || anchor.signature_der.len() > 256
            || anchor.signature_der[0] != 0x30
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        if current_unix_seconds()? > owner.expires_at_unix {
            return Err(VoiceAuthorizationError::OwnerAuthentication);
        }
        let owner_public_key = &owner.owner_public_key_x963;
        let protected_public_key = self.key.public_key(PROMPT_BOOTSTRAP)?;
        if protected_public_key.as_slice() != owner_public_key.as_slice()
            || catalog
                .owner_public_key_x963
                .as_deref()
                .is_some_and(|pinned| pinned != protected_public_key.as_slice())
        {
            return Err(VoiceAuthorizationError::KeyUnavailable);
        }
        if let Some(public_hex) = ledger.value.owner_public_key_x963.as_deref() {
            if decode_lower_hex(public_hex)? != *owner_public_key {
                return Err(VoiceAuthorizationError::KeyUnavailable);
            }
        } else if ledger.value.serial == 0
            && ledger.value.pending.is_none()
            && ledger.value.history.is_empty()
            && ledger.value.active_assets.is_empty()
            && !ledger.has_snapshot_signature()
        {
            ledger.value.owner_public_key_x963 = Some(encode_lower_hex(owner_public_key));
        } else {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        if owner.binding_sha256 != catalog.binding_sha256
            || owner.key_id_sha256 != catalog.key_id_sha256
            || owner.reviewer_roster_sha256 != catalog.reviewer_roster_sha256
            || owner.generation != catalog.generation
        {
            return Err(VoiceAuthorizationError::BindingInvalid);
        }
        let identity = crate::platform::voice_authorization::current_code_identity()?;
        let expected = self.build_owner_binding(catalog, &identity, owner_public_key)?;
        if owner_static_bytes(owner)? != owner_static_bytes(&expected)? {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        parent_auth::verify_owner_binding_for_process(owner, std::process::id() as libc::pid_t)
            .map_err(map_parent_auth_error)?;
        let message = signed_bytes(VOICE_OWNER_BINDING_DOMAIN, owner)?;
        if !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(owner_public_key, &message, &anchor.signature_der)? {
            return Err(VoiceAuthorizationError::OwnerAuthentication);
        }
        Ok(())
    }

    fn owner_payload_from_ledger(
        &self,
        catalog: &FrozenCatalog,
        ledger: &mut LockedLedger,
    ) -> Result<OwnerBindingPayload, VoiceAuthorizationError> {
        let bytes = ledger
            .read_owner_anchor()?
            .ok_or(VoiceAuthorizationError::KeyUnavailable)?;
        let anchor: ParentSigned<OwnerBindingPayload> =
            serde_json::from_slice(&bytes).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
        if canonical_parent_json(&anchor)? != bytes {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        self.verify_owner_anchor(catalog, ledger, &anchor)?;
        ledger.verify_snapshot(&anchor.payload.owner_public_key_x963)?;
        Ok(anchor.payload)
    }
    fn build_owner_binding(
        &self,
        catalog: &FrozenCatalog,
        identity: &crate::platform::voice_authorization::CodeIdentityDigest,
        owner_public_key_x963: &[u8],
    ) -> Result<OwnerBindingPayload, VoiceAuthorizationError> {
        if owner_public_key_x963.len() != 65 || owner_public_key_x963[0] != 0x04 {
            return Err(VoiceAuthorizationError::KeyUnavailable);
        }
        let app_identifier = self.app.config().identifier.clone();
        if identity.bundle_identifier != app_identifier {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let manifest_path = self
            .resource_dir
            .join("saturday-resources/voice-runtime-manifest.json");
        let manifest_metadata = std::fs::symlink_metadata(&manifest_path)
            .map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        if manifest_metadata.file_type().is_symlink()
            || !manifest_metadata.is_file()
            || manifest_metadata.len() > parent_auth::MAX_RUNTIME_MANIFEST_BYTES as u64
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let manifest_file =
            File::open(&manifest_path).map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        let manifest_bytes = read_bounded(&manifest_file, parent_auth::MAX_RUNTIME_MANIFEST_BYTES)?;
        let manifest: RuntimeManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        let mut canonical_manifest = canonical_parent_json(&manifest)?;
        canonical_manifest.push(b'\n');
        if canonical_manifest != manifest_bytes {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let resource_manifest_sha256 = sha256_hex(&manifest_bytes);
        if catalog
            .assets
            .iter()
            .any(|asset| asset.resource_generation_sha256 != resource_manifest_sha256)
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let executable_path =
            std::env::current_exe().map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        let executable =
            File::open(&executable_path).map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        let parent_executable_sha256 = hash_file(&executable)?;
        let parent_resource_closure_sha256 =
            parent_auth::runtime_resource_closure_sha256(&catalog.binding_sha256, &manifest_bytes)
                .map_err(map_parent_auth_error)?;
        let mut workers = catalog
            .assets
            .iter()
            .map(|asset| ParentWorkerBinding {
                source_revision: asset.disclosure.source_revision.clone(),
                role: match asset.worker.role {
                    WorkerRole::Asr => ParentWorkerRole::Asr,
                    WorkerRole::Llm => ParentWorkerRole::Llm,
                },
                asset_id: asset.asset_id.clone(),
                purpose: purpose_name(asset.purpose).to_owned(),
                model_sha256: asset.disclosure.digest_sha256.clone(),
                model_bytes: asset.disclosure.expected_bytes.0,
                source_generation_sha256: asset.source_generation_sha256.clone(),
                source_tree_sha256: asset.source_tree_sha256.clone(),
                resource_generation_sha256: asset.resource_generation_sha256.clone(),
                worker_sha256: asset.worker.sha256.clone(),
                worker_bytes: asset.worker.bytes,
                worker_team_id_sha256: asset.code_identity.worker_team_id_sha256.clone(),
                worker_requirement_sha256: asset.code_identity.worker_requirement_sha256.clone(),
                source_seal_sha256: asset.worker.source_seal_sha256.clone(),
                resource_seal_sha256: asset.worker.resource_seal_sha256.clone(),
                max_rss_bytes: asset.worker.max_rss_bytes,
                template_fingerprint_sha256: asset.template_fingerprint_sha256.clone(),
            })
            .collect::<Vec<_>>();
        workers.sort_by_key(|worker| match worker.role {
            ParentWorkerRole::Asr => 0,
            ParentWorkerRole::Llm => 1,
        });
        let now = current_unix_seconds()?;
        let expires_at_unix = now
            .checked_add(VOICE_PARENT_GRANT_TTL_SECONDS)
            .ok_or(VoiceAuthorizationError::ProtocolFailure)?;
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).map_err(|_| VoiceAuthorizationError::ProtocolFailure)?;
        let owner = OwnerBindingPayload {
            schema: "saturday.voice.owner-binding.v1".to_owned(),
            binding_sha256: catalog.binding_sha256.clone(),
            key_id_sha256: catalog.key_id_sha256.clone(),
            owner_public_key_sha256: sha256_hex(owner_public_key_x963),
            reviewer_roster_sha256: catalog.reviewer_roster_sha256.clone(),
            owner_public_key_x963: owner_public_key_x963.to_vec(),
            generation: catalog.generation,
            application_bundle_identifier: app_identifier,
            application_team_id_sha256: identity.team_id_sha256.clone(),
            application_requirement_sha256: identity.designated_requirement_sha256.clone(),
            parent_executable_sha256,
            parent_resource_closure_sha256,
            resource_manifest_sha256,
            workers,
            issued_at_unix: now,
            expires_at_unix,
            nonce_sha256: sha256_hex(&nonce),
        };
        owner
            .validate()
            .map_err(|_| VoiceAuthorizationError::BindingInvalid)?;
        Ok(owner)
    }
    pub(crate) fn bootstrap_owner_for_current_profile(
        &self,
    ) -> Result<VoiceOwnerBootstrapResult, VoiceAuthorizationError> {
        if is_unprovisioned_bootstrap_profile()? {
            return self
                .bootstrap_key_anchor()
                .map(VoiceOwnerBootstrapResult::BootstrapAnchorCreated);
        }
        self.create_final_owner_binding()
            .map(VoiceOwnerBootstrapResult::FinalBinding)
    }

    fn bootstrap_key_anchor(&self) -> Result<VoiceOwnerBootstrapReview, VoiceAuthorizationError> {
        let identity = crate::platform::voice_authorization::current_code_identity()?;
        if identity.bundle_identifier != self.app.config().identifier {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let ledger = LockedLedger::open_bootstrap(&self.app_data_dir)?;
        let existing = ledger.read_bootstrap_record()?;
        let public_key = match &existing {
            Some(record) => {
                verify_bootstrap_record_for_identity(record, &identity)?;
                let recorded = parent_auth::verify_owner_key_bootstrap_record(record)
                    .map_err(map_parent_auth_error)?;
                let actual = self.key.public_key(PROMPT_BOOTSTRAP)?;
                if actual != recorded {
                    return Err(VoiceAuthorizationError::KeyUnavailable);
                }
                recorded
            }
            None => {
                let body_en = "This creates a dedicated, nonextractable Secure Enclave key for local voice trust only. It does not approve models, record audio, or enable voice inference. The final signed app must independently review and pin every model and worker."
                    .to_owned();
                let body_tr = "Bu işlem yalnızca yerel ses güveni için dışarı aktarılamayan özel bir Secure Enclave anahtarı oluşturur. Modelleri onaylamaz, ses kaydetmez veya ses çıkarımını etkinleştirmez. Son imzalı uygulama her modeli ve çalışanı ayrıca inceleyip sabitlemelidir."
                    .to_owned();
                NativeVoiceConsentPrompt::new(self.app.clone())
                    .confirm_owner_setup(body_en, body_tr)?;

                let public_key = match self.key.public_key(PROMPT_BOOTSTRAP) {
                    Ok(key) => key,
                    Err(VoiceAuthorizationError::KeyUnavailable) => {
                        let mut challenge = [0u8; 32];
                        getrandom::fill(&mut challenge)
                            .map_err(|_| VoiceAuthorizationError::ProtocolFailure)?;
                        let proof = self.key.provision(&challenge, PROMPT_BOOTSTRAP)?;
                        if !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(&proof.public_key_x963, &challenge, &proof.signature_der)? {
                            return Err(VoiceAuthorizationError::OwnerAuthentication);
                        }
                        proof.public_key_x963
                    }
                    Err(error) => return Err(error),
                };
                if public_key.len() != 65 || public_key[0] != 0x04 {
                    return Err(VoiceAuthorizationError::KeyUnavailable);
                }
                let mut nonce = [0u8; 32];
                getrandom::fill(&mut nonce)
                    .map_err(|_| VoiceAuthorizationError::ProtocolFailure)?;
                let payload = OwnerKeyBootstrapPayload {
                    schema: parent_auth::OWNER_KEY_BOOTSTRAP_SCHEMA.to_owned(),
                    application_tag_sha256: sha256_hex(VOICE_KEY_APPLICATION_TAG),
                    owner_public_key_x963: public_key.clone(),
                    owner_public_key_sha256: sha256_hex(&public_key),
                    application_bundle_identifier: identity.bundle_identifier.clone(),
                    application_team_id_sha256: identity.team_id_sha256.clone(),
                    application_requirement_sha256: identity.designated_requirement_sha256.clone(),
                    issued_at_unix: current_unix_seconds()?,
                    nonce_sha256: sha256_hex(&nonce),
                };
                let message = signed_bytes(parent_auth::OWNER_KEY_BOOTSTRAP_DOMAIN, &payload)?;
                let proof = self.key.sign(&message, PROMPT_BOOTSTRAP)?;
                if proof.public_key_x963 != public_key
                    || !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(&public_key, &message, &proof.signature_der)?
                {
                    return Err(VoiceAuthorizationError::OwnerAuthentication);
                }
                let record = canonical_parent_json(&ParentSigned {
                    payload,
                    signature_der: proof.signature_der,
                })?;
                parent_auth::verify_owner_key_bootstrap_record(&record)
                    .map_err(map_parent_auth_error)?;
                ledger.persist_bootstrap_record(&record)?;
                public_key
            }
        };
        let record = ledger
            .read_bootstrap_record()?
            .ok_or(VoiceAuthorizationError::StorageInvalid)?;
        verify_bootstrap_record_for_identity(&record, &identity)?;
        let verified_public_key = parent_auth::verify_owner_key_bootstrap_record(&record)
            .map_err(map_parent_auth_error)?;
        if verified_public_key != public_key || self.key.public_key(PROMPT_BOOTSTRAP)? != public_key
        {
            return Err(VoiceAuthorizationError::KeyUnavailable);
        }
        self.write_bootstrap_witness(&ledger, &record, &public_key)?;
        Ok(VoiceOwnerBootstrapReview {
            owner_public_key_sha256: sha256_hex(&public_key),
            application_bundle_identifier: identity.bundle_identifier,
        })
    }

    fn write_bootstrap_witness(
        &self,
        ledger: &LockedLedger,
        bootstrap_record: &[u8],
        public_key: &[u8],
    ) -> Result<(), VoiceAuthorizationError> {
        let body_en = format!(
            "The app will verify the dedicated Secure Enclave owner key and pin its public key for the signed local workers. No model is approved or downloaded by this step.\nOwner public-key SHA-256: {}",
            sha256_hex(public_key),
        );
        let body_tr = format!(
            "Uygulama, özel Secure Enclave sahibi anahtarını doğrulayacak ve imzalı yerel çalışanlar için açık anahtarını sabitleyecektir. Bu adım hiçbir modeli onaylamaz veya indirmez.\nSahip açık anahtarı SHA-256: {}",
            sha256_hex(public_key),
        );
        NativeVoiceConsentPrompt::new(self.app.clone()).confirm_owner_setup(body_en, body_tr)?;
        let now = current_unix_seconds()?;
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).map_err(|_| VoiceAuthorizationError::ProtocolFailure)?;
        let payload = parent_auth::OwnerKeyBootstrapWitnessPayload {
            schema: parent_auth::OWNER_KEY_BOOTSTRAP_WITNESS_SCHEMA.to_owned(),
            application_tag_sha256: sha256_hex(VOICE_KEY_APPLICATION_TAG),
            owner_public_key_sha256: sha256_hex(public_key),
            bootstrap_record_sha256: sha256_hex(bootstrap_record),
            issued_at_unix: now,
            expires_at_unix: now
                .checked_add(120)
                .ok_or(VoiceAuthorizationError::ProtocolFailure)?,
            nonce_sha256: sha256_hex(&nonce),
        };
        let message = signed_bytes(parent_auth::OWNER_KEY_BOOTSTRAP_WITNESS_DOMAIN, &payload)?;
        let proof = self.key.sign(&message, PROMPT_BOOTSTRAP)?;
        if proof.public_key_x963 != public_key
            || !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(public_key, &message, &proof.signature_der)?
        {
            return Err(VoiceAuthorizationError::OwnerAuthentication);
        }
        let witness = canonical_parent_json(&ParentSigned {
            payload,
            signature_der: proof.signature_der,
        })?;
        parent_auth::verify_owner_key_bootstrap_witness(&witness, bootstrap_record)
            .map_err(map_parent_auth_error)?;
        ledger.persist_bootstrap_witness(&witness)?;
        Ok(())
    }

    fn create_final_owner_binding(&self) -> Result<VoiceBindingReview, VoiceAuthorizationError> {
        let catalog = self.catalog()?;
        let identity = crate::platform::voice_authorization::current_code_identity()?;
        validate_catalog_identity(&catalog, &identity)?;
        let ledger = self.locked(&catalog)?;
        if ledger.read_owner_anchor()?.is_some() {
            return Ok(catalog.review());
        }
        if ledger.value.serial != 0
            || ledger.value.pending.is_some()
            || !ledger.value.history.is_empty()
            || !ledger.value.active_assets.is_empty()
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        let bootstrap_record = ledger
            .read_bootstrap_record()?
            .ok_or(VoiceAuthorizationError::KeyUnavailable)?;
        let witness = ledger
            .read_bootstrap_witness()?
            .ok_or(VoiceAuthorizationError::KeyUnavailable)?;
        parent_auth::verify_owner_key_bootstrap_witness(&witness, &bootstrap_record)
            .map_err(map_parent_auth_error)?;
        let public_key = parent_auth::verify_owner_key_bootstrap_record(&bootstrap_record)
            .map_err(map_parent_auth_error)?;
        if self.key.public_key(PROMPT_BOOTSTRAP)? != public_key
            || catalog.owner_public_key_x963.as_deref() != Some(public_key.as_slice())
        {
            return Err(VoiceAuthorizationError::KeyUnavailable);
        }
        let mut binding = self.build_owner_binding(&catalog, &identity, &public_key)?;
        parent_auth::verify_owner_binding_for_process(&binding, std::process::id() as libc::pid_t)
            .map_err(map_parent_auth_error)?;
        let (body_en, body_tr) = owner_setup_copy(&binding, &catalog);
        NativeVoiceConsentPrompt::new(self.app.clone()).confirm_owner_setup(body_en, body_tr)?;
        binding.owner_public_key_x963 = public_key.clone();
        binding.owner_public_key_sha256 = sha256_hex(&public_key);
        binding
            .validate()
            .map_err(|_| VoiceAuthorizationError::BindingInvalid)?;
        let message = signed_bytes(VOICE_OWNER_BINDING_DOMAIN, &binding)?;
        let proof = self.key.sign(&message, PROMPT_BOOTSTRAP)?;
        if proof.public_key_x963 != public_key
            || !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(&public_key, &message, &proof.signature_der)?
        {
            return Err(VoiceAuthorizationError::OwnerAuthentication);
        }
        let anchor = canonical_parent_json(&ParentSigned {
            payload: binding,
            signature_der: proof.signature_der,
        })?;
        ledger.persist_owner_anchor(&anchor)?;
        Ok(catalog.review())
    }

    pub(crate) fn disclosure(
        &self,
        asset_id: &str,
    ) -> Result<AssetDisclosure, VoiceAuthorizationError> {
        let catalog = self.catalog()?;
        Ok(catalog.asset(asset_id)?.disclosure.clone())
    }

    pub(crate) fn active_receipt(
        &self,
        asset_id: &str,
    ) -> Result<Option<ActiveAssetReceipt>, VoiceAuthorizationError> {
        let catalog = self.catalog()?;
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        Ok(ledger
            .value
            .active_assets
            .iter()
            .find(|receipt| receipt.asset_id == asset_id)
            .cloned())
    }

    pub(crate) fn pending_transfers(
        &self,
    ) -> Result<Vec<VoiceTransferLease>, VoiceAuthorizationError> {
        let catalog = self.catalog()?;
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let Some(record) = &ledger.value.pending else {
            return Ok(Vec::new());
        };
        if !matches!(
            record.state,
            GrantState::Issued | GrantState::Consuming | GrantState::PurposeBound
        ) {
            return Ok(Vec::new());
        }
        let payload = &record.payload;
        Ok(vec![VoiceTransferLease {
            generation: payload.generation,
            grant_id: decode_lower_hex(&payload.grant_id_hex)?
                .try_into()
                .map_err(|_| VoiceAuthorizationError::StorageInvalid)?,
            asset_id: payload.asset_id.clone(),
            purpose: parse_purpose(&payload.purpose)?,
            expected_bytes: payload.expected_bytes,
            expected_sha256: payload.expected_sha256.clone(),
            binding_sha256: payload.binding_sha256.clone(),
            expires_at_unix: payload.expires_at_unix,
        }])
    }

    pub(crate) fn resolve_interrupted(
        &self,
        asset_id: &str,
        generation: u64,
    ) -> Result<(), VoiceAuthorizationError> {
        let catalog = self.catalog()?;
        catalog.asset(asset_id)?;
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let record = ledger
            .value
            .pending
            .as_mut()
            .ok_or(VoiceAuthorizationError::RecoveryRequired)?;
        if record.payload.asset_id != asset_id
            || record.payload.generation != generation
            || !matches!(
                record.state,
                GrantState::Issued | GrantState::Consuming | GrantState::PurposeBound
            )
        {
            return Err(VoiceAuthorizationError::GrantConsumed);
        }
        record.state = GrantState::Interrupted;
        ledger.value.archive_terminal_pending()?;
        self.persist_signed_ledger(&mut ledger)
    }

    pub(crate) fn authorize_model_purpose(
        &self,
        caller: &NativeCallerContext,
        candidate: &VerifiedModelFile,
        cancellation: &CancelToken,
    ) -> Result<(), VoiceAuthorizationError> {
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        validate_live_caller(&self.app, caller)?;
        let catalog = self.catalog()?;
        let asset = catalog.asset(candidate.asset_id())?;
        if candidate.purpose() != AssetPurpose::LlmWeights
            || asset.purpose != AssetPurpose::LlmWeights
            || asset.disclosure.digest_sha256 != candidate.payload_sha256()
            || candidate.bytes() != asset.disclosure.expected_bytes.0
            || candidate.version() != candidate.payload_sha256()
            || candidate.template_sha256().is_none()
            || candidate.tokenizer_metadata_sha256().is_none()
        {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        validate_verified_model_file(candidate, asset)?;
        let inspected =
            crate::voice_authorization::gguf::inspect_qwen3(candidate.file(), candidate.bytes())?;
        if candidate.template_sha256() != Some(&inspected.chat_template_sha256)
            || candidate.tokenizer_metadata_sha256() != Some(&inspected.tokenizer_metadata_sha256)
        {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let record = ledger
            .value
            .pending
            .as_ref()
            .ok_or(VoiceAuthorizationError::GrantInvalid)?;
        if record.payload.asset_id != candidate.asset_id()
            || record.payload.expected_sha256 != candidate.payload_sha256()
            || record.payload.purpose != "llm_weights"
            || record.state != GrantState::Consuming
            || current_unix_seconds()? > record.payload.expires_at_unix
        {
            return Err(VoiceAuthorizationError::GrantExpired);
        }
        let prompt = super::consent::NativeVoiceConsentPrompt::new(self.app.clone());
        let consent = prompt.confirm_model_purpose(caller, candidate, cancellation)?;
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        validate_live_caller(&self.app, caller)?;
        let purpose_bytes = model_purpose_bytes(&asset.disclosure, candidate)?;
        let purpose_nonce = consent.nonce;
        let disclosure_sha256: [u8; 32] = Sha256::digest(&purpose_bytes).into();
        if consent.scope
            != (OwnerConsentScope::ModelPurpose {
                asset_id: candidate.asset_id().to_owned(),
                model_sha256: candidate.payload_sha256().to_owned(),
                template_sha256: sha256_hex(&inspected.chat_template_sha256),
                tokenizer_metadata_sha256: sha256_hex(&inspected.tokenizer_metadata_sha256),
            })
            || consent.disclosure_sha256 != disclosure_sha256
        {
            return Err(VoiceAuthorizationError::GrantInvalid);
        }
        let purpose_nonce_sha256 = sha256_hex(&purpose_nonce);
        if nonce_already_used(&ledger.value, &purpose_nonce_sha256) {
            return Err(VoiceAuthorizationError::GrantConsumed);
        }
        let serial = ledger
            .value
            .serial
            .checked_add(1)
            .ok_or(VoiceAuthorizationError::ProtocolFailure)?;
        let payload = PurposePayload {
            schema: PURPOSE_SCHEMA.to_owned(),
            serial,
            asset_id: candidate.asset_id().to_owned(),
            model_sha256: candidate.payload_sha256().to_owned(),
            template_sha256: sha256_hex(&inspected.chat_template_sha256),
            tokenizer_metadata_sha256: sha256_hex(&inspected.tokenizer_metadata_sha256),
            architecture: inspected.architecture,
            context_length: inspected.context_length,
            binding_sha256: record.payload.binding_sha256.clone(),
            source_generation_sha256: record.payload.source_generation_sha256.clone(),
            source_tree_sha256: record.payload.source_tree_sha256.clone(),
            resource_generation_sha256: record.payload.resource_generation_sha256.clone(),
            worker_sha256: record.payload.worker_sha256.clone(),
            worker_source_seal_sha256: record.payload.worker_source_seal_sha256.clone(),
            worker_resource_seal_sha256: record.payload.worker_resource_seal_sha256.clone(),
            application_bundle_identifier: record.payload.application_bundle_identifier.clone(),
            application_team_id_sha256: record.payload.application_team_id_sha256.clone(),
            application_requirement_sha256: record.payload.application_requirement_sha256.clone(),
            parent_executable_sha256: record.payload.parent_executable_sha256.clone(),
            parent_resource_closure_sha256: record.payload.parent_resource_closure_sha256.clone(),
            worker_team_id_sha256: record.payload.worker_team_id_sha256.clone(),
            worker_requirement_sha256: record.payload.worker_requirement_sha256.clone(),
            issued_at_unix: current_unix_seconds()?,
            expires_at_unix: record.payload.use_expires_at_unix,
            consent_nonce_sha256: purpose_nonce_sha256.clone(),
        };
        let signing_bytes = signed_bytes(VOICE_MODEL_PURPOSE_DOMAIN, &payload)?;
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let proof = self.key.sign(&signing_bytes, PROMPT_PURPOSE)?;
        verify_proof(self.key.as_ref(), &ledger.value, &signing_bytes, &proof)?;
        validate_live_caller(&self.app, caller)?;
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let record = ledger
            .value
            .pending
            .as_mut()
            .ok_or(VoiceAuthorizationError::GrantInvalid)?;
        if record.payload.asset_id != candidate.asset_id()
            || record.payload.expected_sha256 != candidate.payload_sha256()
            || record.state != GrantState::Consuming
        {
            return Err(VoiceAuthorizationError::GrantInvalid);
        }
        record.purpose_proof = Some(PurposeRecord {
            payload,
            signature_der: proof.signature_der,
        });
        record.state = GrantState::PurposeBound;
        ledger.value.serial = serial;
        ledger.value.last_nonce_sha256 = Some(purpose_nonce_sha256);
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let authorization_state = self.signed_ledger_snapshot(&mut ledger)?;
        validate_live_caller(&self.app, caller)?;
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let _commit = cancellation
            .begin_commit()
            .ok_or(VoiceAuthorizationError::OwnerCancelled)?;
        ledger.persist(&authorization_state)?;
        Ok(())
    }

    pub(crate) fn verify_worker_bundle(
        &self,
        role: WorkerRole,
    ) -> Result<(), VoiceAuthorizationError> {
        let catalog = self.catalog()?;
        let asset = catalog.asset_by_purpose(match role {
            WorkerRole::Asr => AssetPurpose::AsrWeights,
            WorkerRole::Llm => AssetPurpose::LlmWeights,
        })?;
        if asset.worker.role != role {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let mut ledger = self.locked(&catalog)?;
        let owner = self.owner_payload_from_ledger(&catalog, &mut ledger)?;
        let app_identity = crate::platform::voice_authorization::current_code_identity()?;
        if asset.code_identity.application_team_id_sha256 != app_identity.team_id_sha256
            || asset.code_identity.application_requirement_sha256
                != app_identity.designated_requirement_sha256
            || owner.application_bundle_identifier != app_identity.bundle_identifier
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        parent_auth::verify_owner_binding_for_process(&owner, std::process::id() as libc::pid_t)
            .map_err(map_parent_auth_error)?;
        let worker_path = self
            .resource_dir
            .join(&asset.worker.executable_relative_path);
        let metadata = std::fs::symlink_metadata(&worker_path)
            .map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != asset.worker.bytes
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let worker_identity = crate::platform::voice_authorization::code_identity_at(&worker_path)?;
        if worker_identity.team_id_sha256 != asset.code_identity.worker_team_id_sha256
            || worker_identity.designated_requirement_sha256
                != asset.code_identity.worker_requirement_sha256
            || worker_identity.team_id_sha256 != app_identity.team_id_sha256
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let file = super::ledger::open_relative_readonly(
            &self.resource_dir,
            Path::new(&asset.worker.executable_relative_path),
            asset.worker.bytes,
            false,
        )?;
        if hash_file(&file)? != asset.worker.sha256 {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        Ok(())
    }
    pub(crate) fn approve_model_file(
        &self,
        candidate: VerifiedModelFile,
    ) -> Result<ApprovedModelHandle, VoiceAuthorizationError> {
        let catalog = self.catalog()?;
        let asset = catalog.asset(candidate.asset_id())?;
        if candidate.purpose() != asset.purpose
            || candidate.payload_sha256() != asset.disclosure.digest_sha256
            || candidate.version() != candidate.payload_sha256()
            || candidate.bytes() != asset.disclosure.expected_bytes.0
        {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let receipt = ledger
            .value
            .active_assets
            .iter()
            .find(|receipt| receipt.asset_id == candidate.asset_id())
            .ok_or(VoiceAuthorizationError::AssetMismatch)?;
        if receipt.state != AuthorizedAssetState::Enabled
            || receipt.sha256 != candidate.payload_sha256()
            || receipt.version != candidate.version()
            || receipt.bytes != candidate.bytes()
        {
            return Err(VoiceAuthorizationError::AssetMismatch);
        }
        validate_verified_model_file(&candidate, asset)?;
        let relative = model_payload_relative_path(asset);
        let file = super::ledger::open_relative_readonly(
            &self.app_data_dir,
            &relative,
            asset.disclosure.expected_bytes.0,
            true,
        )?;
        let metadata = file
            .metadata()
            .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
        if metadata.dev() != candidate.device
            || metadata.ino() != candidate.inode
            || hash_file(&file)? != receipt.sha256
        {
            return Err(VoiceAuthorizationError::ModelInvalid);
        }
        let (template, tokenizer) = if asset.purpose == AssetPurpose::LlmWeights {
            let inspected = crate::voice_authorization::gguf::inspect_qwen3(
                &file,
                asset.disclosure.expected_bytes.0,
            )?;
            let template = sha256_hex(&inspected.chat_template_sha256);
            let tokenizer = sha256_hex(&inspected.tokenizer_metadata_sha256);
            if receipt.template_fingerprint_sha256.as_deref() != Some(template.as_str())
                || receipt.tokenizer_metadata_sha256.as_deref() != Some(tokenizer.as_str())
                || candidate.template_sha256() != Some(&inspected.chat_template_sha256)
                || candidate.tokenizer_metadata_sha256()
                    != Some(&inspected.tokenizer_metadata_sha256)
            {
                return Err(VoiceAuthorizationError::ModelInvalid);
            }
            (Some(template), Some(tokenizer))
        } else {
            (None, None)
        };
        let identity = VoiceModelIdentity {
            asset_id: asset.asset_id.clone(),
            version: receipt.version.clone(),
            purpose: asset.purpose,
            bytes: receipt.bytes,
            sha256: receipt.sha256.clone(),
            chat_template_sha256: template,
            tokenizer_metadata_sha256: tokenizer,
            binding_sha256: catalog.binding_sha256.clone(),
        };
        let sandbox_path = model_sandbox_path(asset);
        ApprovedModelHandle::from_verified_binding(file, identity, sandbox_path)
    }

    pub(crate) fn approve_worker_closure(
        &self,
        role: WorkerRole,
    ) -> Result<ApprovedRuntimeClosure, VoiceAuthorizationError> {
        let parent_role = match role {
            WorkerRole::Asr => ParentWorkerRole::Asr,
            WorkerRole::Llm => ParentWorkerRole::Llm,
        };
        let catalog = self.catalog()?;
        let asset = catalog.asset_by_purpose(match role {
            WorkerRole::Asr => AssetPurpose::AsrWeights,
            WorkerRole::Llm => AssetPurpose::LlmWeights,
        })?;
        if asset.worker.role != role {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let owner = self.owner_payload_from_ledger(&catalog, &mut ledger)?;
        parent_auth::verify_owner_binding_for_process(&owner, std::process::id() as libc::pid_t)
            .map_err(map_parent_auth_error)?;

        let receipt = ledger
            .value
            .active_assets
            .iter()
            .find(|receipt| receipt.asset_id == asset.asset_id)
            .ok_or(VoiceAuthorizationError::AssetMismatch)?;
        if receipt.state != AuthorizedAssetState::Enabled
            || receipt.purpose != purpose_name(asset.purpose)
            || receipt.bytes != asset.disclosure.expected_bytes.0
            || receipt.sha256 != asset.disclosure.digest_sha256
            || receipt.version != receipt.sha256
        {
            return Err(VoiceAuthorizationError::AssetMismatch);
        }
        let grant_record = ledger
            .value
            .history
            .iter()
            .chain(ledger.value.pending.iter())
            .find(|record| {
                record.payload.generation == receipt.generation
                    && record.payload.asset_id == receipt.asset_id
                    && record.state == GrantState::Promoted
                    && record.promoted_version.as_deref() == Some(receipt.version.as_str())
            })
            .ok_or(VoiceAuthorizationError::GrantInvalid)?;
        if grant_record.payload.expected_sha256 != receipt.sha256
            || grant_record.payload.expected_bytes != receipt.bytes
            || grant_record.payload.binding_sha256 != catalog.binding_sha256
            || grant_record.payload.worker_role != role
            || current_unix_seconds()? > grant_record.payload.use_expires_at_unix
        {
            return Err(VoiceAuthorizationError::GrantExpired);
        }
        let purpose_proof = match (role, grant_record.purpose_proof.as_ref()) {
            (WorkerRole::Asr, None) => None,
            (WorkerRole::Asr, Some(_)) | (WorkerRole::Llm, None) => {
                return Err(VoiceAuthorizationError::GrantInvalid);
            }
            (WorkerRole::Llm, Some(proof)) => {
                if receipt.template_fingerprint_sha256.as_deref()
                    != Some(proof.payload.template_sha256.as_str())
                    || receipt.tokenizer_metadata_sha256.as_deref()
                        != Some(proof.payload.tokenizer_metadata_sha256.as_str())
                    || proof.payload.model_sha256 != receipt.sha256
                    || proof.payload.expires_at_unix < grant_record.payload.use_expires_at_unix
                {
                    return Err(VoiceAuthorizationError::ModelInvalid);
                }
                Some(ParentSigned {
                    payload: ParentPurposePayload {
                        schema: proof.payload.schema.clone(),
                        serial: proof.payload.serial,
                        asset_id: proof.payload.asset_id.clone(),
                        model_sha256: proof.payload.model_sha256.clone(),
                        template_sha256: proof.payload.template_sha256.clone(),
                        tokenizer_metadata_sha256: proof.payload.tokenizer_metadata_sha256.clone(),
                        architecture: proof.payload.architecture.clone(),
                        context_length: proof.payload.context_length,
                        binding_sha256: proof.payload.binding_sha256.clone(),
                        source_generation_sha256: proof.payload.source_generation_sha256.clone(),
                        source_tree_sha256: proof.payload.source_tree_sha256.clone(),
                        resource_generation_sha256: proof
                            .payload
                            .resource_generation_sha256
                            .clone(),
                        worker_sha256: proof.payload.worker_sha256.clone(),
                        worker_source_seal_sha256: proof.payload.worker_source_seal_sha256.clone(),
                        worker_resource_seal_sha256: proof
                            .payload
                            .worker_resource_seal_sha256
                            .clone(),
                        application_bundle_identifier: proof
                            .payload
                            .application_bundle_identifier
                            .clone(),
                        application_team_id_sha256: proof
                            .payload
                            .application_team_id_sha256
                            .clone(),
                        application_requirement_sha256: proof
                            .payload
                            .application_requirement_sha256
                            .clone(),
                        parent_executable_sha256: proof.payload.parent_executable_sha256.clone(),
                        parent_resource_closure_sha256: proof
                            .payload
                            .parent_resource_closure_sha256
                            .clone(),
                        worker_team_id_sha256: proof.payload.worker_team_id_sha256.clone(),
                        worker_requirement_sha256: proof.payload.worker_requirement_sha256.clone(),
                        issued_at_unix: proof.payload.issued_at_unix,
                        expires_at_unix: proof.payload.expires_at_unix,
                        consent_nonce_sha256: proof.payload.consent_nonce_sha256.clone(),
                    },
                    signature_der: proof.signature_der.clone(),
                })
            }
        };

        let transfer = &grant_record.payload;
        let owner_binding = {
            let bytes = ledger
                .read_owner_anchor()?
                .ok_or(VoiceAuthorizationError::KeyUnavailable)?;
            serde_json::from_slice(&bytes).map_err(|_| VoiceAuthorizationError::StorageInvalid)?
        };
        let parent_grant = ParentWorkerGrant {
            schema: saturday_voice_parent_auth::PARENT_GRANT_SCHEMA.to_owned(),
            worker_role: parent_role,
            owner_binding,
            asset_grant: ParentSigned {
                payload: ParentTransferGrantPayload {
                    schema: transfer.schema.clone(),
                    request_role: transfer.request_role.clone(),
                    generation: transfer.generation,
                    grant_id_hex: transfer.grant_id_hex.clone(),
                    asset_id: transfer.asset_id.clone(),
                    purpose: transfer.purpose.clone(),
                    worker_role: parent_role,
                    expected_bytes: transfer.expected_bytes,
                    expected_sha256: transfer.expected_sha256.clone(),
                    binding_sha256: transfer.binding_sha256.clone(),
                    source_generation_sha256: transfer.source_generation_sha256.clone(),
                    source_tree_sha256: transfer.source_tree_sha256.clone(),
                    resource_generation_sha256: transfer.resource_generation_sha256.clone(),
                    worker_sha256: transfer.worker_sha256.clone(),
                    worker_bytes: transfer.worker_bytes,
                    worker_source_seal_sha256: transfer.worker_source_seal_sha256.clone(),
                    worker_resource_seal_sha256: transfer.worker_resource_seal_sha256.clone(),
                    template_fingerprint_sha256: transfer.template_fingerprint_sha256.clone(),
                    application_bundle_identifier: transfer.application_bundle_identifier.clone(),
                    application_team_id_sha256: transfer.application_team_id_sha256.clone(),
                    application_requirement_sha256: transfer.application_requirement_sha256.clone(),
                    parent_executable_sha256: transfer.parent_executable_sha256.clone(),
                    parent_resource_closure_sha256: transfer.parent_resource_closure_sha256.clone(),
                    worker_team_id_sha256: transfer.worker_team_id_sha256.clone(),
                    worker_requirement_sha256: transfer.worker_requirement_sha256.clone(),
                    issued_at_unix: transfer.issued_at_unix,
                    expires_at_unix: transfer.expires_at_unix,
                    use_expires_at_unix: transfer.use_expires_at_unix,
                    consent_nonce_sha256: transfer.consent_nonce_sha256.clone(),
                },
                signature_der: grant_record.signature_der.clone(),
            },
            model_purpose: purpose_proof,
            authorization_state: ledger.authorization_state()?.clone(),
            launch_authorization: None,
        };

        let manifest_path = Path::new("saturday-resources/voice-runtime-manifest.json");
        let manifest_metadata = std::fs::symlink_metadata(self.resource_dir.join(manifest_path))
            .map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        if manifest_metadata.file_type().is_symlink()
            || !manifest_metadata.is_file()
            || manifest_metadata.len() == 0
            || manifest_metadata.len()
                > saturday_voice_parent_auth::MAX_RUNTIME_MANIFEST_BYTES as u64
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let manifest_file = super::ledger::open_relative_readonly(
            &self.resource_dir,
            manifest_path,
            manifest_metadata.len(),
            false,
        )?;
        let manifest_bytes = read_bounded(
            &manifest_file,
            saturday_voice_parent_auth::MAX_RUNTIME_MANIFEST_BYTES,
        )?;
        if sha256_hex(&manifest_bytes) != owner.resource_manifest_sha256 {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let manifest: RuntimeManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        let manifest_worker = manifest
            .workers
            .iter()
            .find(|worker| worker.role == parent_role)
            .ok_or(VoiceAuthorizationError::WorkerMismatch)?;
        if manifest_worker.path != asset.worker.executable_relative_path
            || manifest_worker.sha256 != asset.worker.sha256
            || manifest_worker.bytes != asset.worker.bytes
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let app_identity = crate::platform::voice_authorization::current_code_identity()?;
        if asset.code_identity.application_team_id_sha256 != app_identity.team_id_sha256
            || asset.code_identity.application_requirement_sha256
                != app_identity.designated_requirement_sha256
            || owner.application_bundle_identifier != app_identity.bundle_identifier
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let worker_path = self.resource_dir.join(&manifest_worker.path);
        let worker_identity = crate::platform::voice_authorization::code_identity_at(&worker_path)?;
        if worker_identity.team_id_sha256 != asset.code_identity.worker_team_id_sha256
            || worker_identity.designated_requirement_sha256
                != asset.code_identity.worker_requirement_sha256
            || worker_identity.team_id_sha256 != app_identity.team_id_sha256
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let file = super::ledger::open_relative_readonly(
            &self.resource_dir,
            Path::new(&manifest_worker.path),
            asset.worker.bytes,
            false,
        )?;
        if hash_file(&file)? != asset.worker.sha256 {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        let mut parent_resources = std::collections::BTreeSet::new();
        parent_resources.insert(
            self.resource_dir
                .join("saturday-resources/voice-asset-bindings.json"),
        );
        parent_resources.insert(self.resource_dir.join(manifest_path));
        for worker in &manifest.workers {
            parent_resources.insert(self.resource_dir.join(&worker.path));
        }
        for resource in &manifest.resources {
            parent_resources.insert(self.resource_dir.join(&resource.path));
        }
        let parent_resource_files = parent_resources.into_iter().collect::<Vec<_>>();
        ApprovedRuntimeClosure::from_verified_binding(RuntimeClosureBinding {
            role,
            executable: file,
            executable_path: worker_path,
            worker_sha256: asset.worker.sha256.clone(),
            worker_bytes: asset.worker.bytes,
            worker_bundle_identifier: worker_identity.bundle_identifier,
            worker_team_id_sha256: worker_identity.team_id_sha256,
            worker_requirement_sha256: worker_identity.designated_requirement_sha256,
            source_seal_sha256: asset.worker.source_seal_sha256.clone(),
            resource_seal_sha256: asset.worker.resource_seal_sha256.clone(),
            max_rss_bytes: asset.worker.max_rss_bytes,
            binding_sha256: catalog.binding_sha256.clone(),
            parent_grant,
            authorization_state: canonical_parent_json(ledger.authorization_state()?)?,
            parent_resource_files,
        })
    }

    pub(crate) fn disable_asset(&self, asset_id: &str) -> Result<(), VoiceAuthorizationError> {
        let catalog = self.catalog()?;
        catalog.asset(asset_id)?;
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let receipt = ledger
            .value
            .active_assets
            .iter_mut()
            .find(|receipt| receipt.asset_id == asset_id)
            .ok_or(VoiceAuthorizationError::AssetMismatch)?;
        if receipt.state == AuthorizedAssetState::Enabled {
            receipt.state = AuthorizedAssetState::Disabled;
            self.persist_signed_ledger(&mut ledger)?;
        }
        Ok(())
    }

    pub(crate) fn delete_asset(&self, asset_id: &str) -> Result<(), VoiceAuthorizationError> {
        let catalog = self.catalog()?;
        catalog.asset(asset_id)?;
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let receipt = ledger
            .value
            .active_assets
            .iter_mut()
            .find(|receipt| receipt.asset_id == asset_id)
            .ok_or(VoiceAuthorizationError::AssetMismatch)?;
        if receipt.state != AuthorizedAssetState::Revoked {
            receipt.state = AuthorizedAssetState::Revoked;
            self.persist_signed_ledger(&mut ledger)?;
        }
        Ok(())
    }

    pub(crate) fn rollback_asset(
        &self,
        asset_id: &str,
        version: &str,
    ) -> Result<ActiveAssetReceipt, VoiceAuthorizationError> {
        if !is_sha256(version) {
            return Err(VoiceAuthorizationError::AssetMismatch);
        }
        let catalog = self.catalog()?;
        catalog.asset(asset_id)?;
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let old = ledger
            .value
            .history
            .iter()
            .rev()
            .find(|record| {
                record.state == GrantState::Promoted
                    && record.payload.asset_id == asset_id
                    && record.payload.expected_sha256 == version
            })
            .cloned()
            .ok_or(VoiceAuthorizationError::AssetMismatch)?;
        let receipt = ledger
            .value
            .active_assets
            .iter_mut()
            .find(|receipt| receipt.asset_id == asset_id)
            .ok_or(VoiceAuthorizationError::AssetMismatch)?;
        receipt.version = old.payload.expected_sha256.clone();
        receipt.sha256 = old.payload.expected_sha256.clone();
        receipt.bytes = old.payload.expected_bytes;
        receipt.generation = old.payload.generation;
        receipt.template_fingerprint_sha256 = old
            .purpose_proof
            .as_ref()
            .map(|proof| proof.payload.template_sha256.clone());
        receipt.tokenizer_metadata_sha256 = old
            .purpose_proof
            .as_ref()
            .map(|proof| proof.payload.tokenizer_metadata_sha256.clone());
        receipt.state = AuthorizedAssetState::Enabled;
        let result = receipt.clone();
        self.persist_signed_ledger(&mut ledger)?;
        Ok(result)
    }
    fn session_snapshot_is_current(
        &self,
        session: &AuthorizedWorkerSession,
    ) -> Result<bool, VoiceAuthorizationError> {
        let catalog = self.catalog()?;
        if catalog.binding_sha256 != session.binding_sha256 {
            return Ok(false);
        }
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let owner = self.owner_payload_from_ledger(&catalog, &mut ledger)?;
        let public_key = decode_lower_hex(
            ledger
                .value
                .owner_public_key_x963
                .as_deref()
                .ok_or(VoiceAuthorizationError::KeyUnavailable)?,
        )?;
        let executable_path =
            std::env::current_exe().map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        let executable =
            File::open(executable_path).map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        let current_parent_executable_sha256 = hash_file(&executable)?;
        let manifest_path = self
            .resource_dir
            .join("saturday-resources/voice-runtime-manifest.json");
        let manifest_file =
            File::open(&manifest_path).map_err(|_| VoiceAuthorizationError::WorkerMismatch)?;
        let manifest_bytes = read_bounded(&manifest_file, parent_auth::MAX_RUNTIME_MANIFEST_BYTES)?;
        let current_closure =
            parent_auth::runtime_resource_closure_sha256(&catalog.binding_sha256, &manifest_bytes)
                .map_err(map_parent_auth_error)?;
        if !session_state_serial_is_current(session.state_serial, ledger.value.serial)
            || owner.binding_sha256 != session.binding_sha256
            || sha256_hex(&public_key) != session.owner_public_key_sha256
            || current_parent_executable_sha256 != session.parent_executable_sha256
            || current_closure != session.parent_resource_closure_sha256
            || owner.parent_executable_sha256 != session.parent_executable_sha256
            || owner.parent_resource_closure_sha256 != session.parent_resource_closure_sha256
        {
            return Ok(false);
        }
        for asset in &session.assets {
            let installed = ledger.value.active_assets.iter().any(|receipt| {
                receipt.asset_id == asset.asset_id
                    && receipt.generation == asset.generation
                    && receipt.state == AuthorizedAssetState::Enabled
                    && receipt.sha256 == asset.sha256
                    && catalog.asset(&asset.asset_id).is_ok_and(|frozen| {
                        frozen.worker.role == asset.role
                            && frozen.disclosure.digest_sha256 == asset.sha256
                    })
            });
            if !installed {
                return Ok(false);
            }
        }
        drop(ledger);
        for asset in &session.assets {
            self.approve_worker_closure(asset.role)?;
        }
        Ok(true)
    }
}

impl VoiceAssetAuthority for VoiceAuthorizationService {
    fn authorize_install(
        &self,
        caller: &NativeCallerContext,
        asset_id: &str,
        disclosure: &AssetDisclosure,
        consent: OwnerConsent,
        cancellation: &CancelToken,
    ) -> Result<VoiceGrant, VoiceAuthorizationError> {
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        validate_live_caller(&self.app, caller)?;
        let catalog = self.catalog()?;
        let asset = catalog.asset(asset_id)?;
        let bytes =
            serde_json::to_vec(disclosure).map_err(|_| VoiceAuthorizationError::BindingInvalid)?;
        if &asset.disclosure != disclosure
            || bytes.len() > super::MAX_DISCLOSURE_BYTES
            || consent.scope
                != (OwnerConsentScope::Download {
                    asset_id: asset_id.to_owned(),
                })
            || !consent.matches(
                &bytes,
                caller,
                &OwnerConsentScope::Download {
                    asset_id: asset_id.to_owned(),
                },
            )
        {
            return Err(VoiceAuthorizationError::AssetMismatch);
        }
        let app_identity = crate::platform::voice_authorization::current_code_identity()?;
        validate_catalog_identity(&catalog, &app_identity)?;
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let owner = self.owner_payload_from_ledger(&catalog, &mut ledger)?;
        if ledger.value.owner_public_key_x963.is_none() {
            return Err(VoiceAuthorizationError::KeyUnavailable);
        }
        if ledger.value.pending.is_some() {
            return Err(VoiceAuthorizationError::RecoveryRequired);
        }
        let consent_nonce_sha256 = sha256_hex(&consent.nonce);
        if nonce_already_used(&ledger.value, &consent_nonce_sha256) {
            return Err(VoiceAuthorizationError::GrantConsumed);
        }
        let serial = ledger
            .value
            .serial
            .checked_add(1)
            .ok_or(VoiceAuthorizationError::ProtocolFailure)?;
        let now = current_unix_seconds()?;
        let expiry = now
            .checked_add(VOICE_TRANSFER_TTL_SECONDS)
            .ok_or(VoiceAuthorizationError::ProtocolFailure)?
            .min(owner.expires_at_unix);
        let use_expiry = now
            .checked_add(VOICE_PARENT_GRANT_TTL_SECONDS)
            .ok_or(VoiceAuthorizationError::ProtocolFailure)?
            .min(owner.expires_at_unix);
        if expiry <= now || use_expiry <= now {
            return Err(VoiceAuthorizationError::OwnerAuthentication);
        }
        let mut grant_id = [0; 16];
        getrandom::fill(&mut grant_id).map_err(|_| VoiceAuthorizationError::ProtocolFailure)?;
        if grant_id == [0; 16]
            || ledger
                .value
                .history
                .iter()
                .any(|record| record.payload.grant_id_hex == encode_lower_hex(&grant_id))
        {
            return Err(VoiceAuthorizationError::ProtocolFailure);
        }
        let asset = catalog.asset(asset_id)?;
        let payload = grant_payload(GrantRequest {
            asset,
            catalog: &catalog,
            generation: serial,
            grant_id: &grant_id,
            issued_at: now,
            expires_at: expiry,
            nonce: &consent.nonce,
            owner: &owner,
            use_expires_at: use_expiry,
        })?;
        let signing_bytes = signed_bytes(VOICE_GRANT_DOMAIN, &payload)?;
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let proof = self.key.sign(&signing_bytes, PROMPT_GRANT)?;
        verify_proof(self.key.as_ref(), &ledger.value, &signing_bytes, &proof)?;
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        validate_live_caller(&self.app, caller)?;
        ledger.value.serial = serial;
        ledger.value.last_nonce_sha256 = Some(consent_nonce_sha256);
        ledger.value.pending = Some(GrantRecord {
            payload: payload.clone(),
            signature_der: proof.signature_der.clone(),
            state: GrantState::Issued,
            purpose_proof: None,
            promoted_version: None,
        });
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let authorization_state = self.signed_ledger_snapshot(&mut ledger)?;
        validate_live_caller(&self.app, caller)?;
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let _commit = cancellation
            .begin_commit()
            .ok_or(VoiceAuthorizationError::OwnerCancelled)?;
        ledger.persist(&authorization_state)?;
        Ok(VoiceGrant {
            generation: serial,
            grant_id,
            asset_id: payload.asset_id,
            purpose: asset.purpose,
            expected_bytes: payload.expected_bytes,
            expected_sha256: payload.expected_sha256,
            binding_sha256: catalog.binding_sha256,
            issued_at_unix: now,
            expires_at_unix: expiry,
            signature_der: proof.signature_der,
            consent_nonce: consent.nonce,
        })
    }

    fn authorize_model_purpose(
        &self,
        caller: &NativeCallerContext,
        candidate: &VerifiedModelFile,
        cancellation: &CancelToken,
    ) -> Result<(), VoiceAuthorizationError> {
        VoiceAuthorizationService::authorize_model_purpose(self, caller, candidate, cancellation)
    }

    fn begin_transfer(
        &self,
        grant: VoiceGrant,
        cancellation: &CancelToken,
    ) -> Result<VoiceTransferLease, VoiceAuthorizationError> {
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let catalog = self.catalog()?;
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let record = ledger
            .value
            .pending
            .as_mut()
            .ok_or(VoiceAuthorizationError::GrantInvalid)?;
        if record.state != GrantState::Issued
            || record.payload.generation != grant.generation
            || record.payload.grant_id_hex != encode_lower_hex(&grant.grant_id)
            || record.payload.asset_id != grant.asset_id
            || record.payload.purpose != purpose_name(grant.purpose)
            || record.payload.expected_bytes != grant.expected_bytes
            || record.payload.expected_sha256 != grant.expected_sha256
            || record.payload.binding_sha256 != grant.binding_sha256
            || record.payload.issued_at_unix != grant.issued_at_unix
            || record.payload.expires_at_unix != grant.expires_at_unix
            || record.payload.consent_nonce_sha256 != sha256_hex(&grant.consent_nonce)
            || record.signature_der != grant.signature_der
        {
            return Err(VoiceAuthorizationError::GrantConsumed);
        }
        if current_unix_seconds()? > record.payload.expires_at_unix {
            if cancellation.is_cancelled() {
                return Err(VoiceAuthorizationError::OwnerCancelled);
            }
            record.state = GrantState::Failed;
            ledger.value.archive_terminal_pending()?;
            if cancellation.is_cancelled() {
                return Err(VoiceAuthorizationError::OwnerCancelled);
            }
            let signature_der = self.signed_ledger_snapshot(&mut ledger)?;
            if cancellation.is_cancelled() {
                return Err(VoiceAuthorizationError::OwnerCancelled);
            }
            let _commit = cancellation
                .begin_commit()
                .ok_or(VoiceAuthorizationError::OwnerCancelled)?;
            ledger.persist(&signature_der)?;
            return Err(VoiceAuthorizationError::GrantExpired);
        }
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        record.state = GrantState::Consuming;
        let lease = VoiceTransferLease {
            generation: grant.generation,
            grant_id: grant.grant_id,
            asset_id: grant.asset_id,
            purpose: grant.purpose,
            expected_bytes: grant.expected_bytes,
            expected_sha256: grant.expected_sha256,
            binding_sha256: grant.binding_sha256,
            expires_at_unix: grant.expires_at_unix,
        };
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let signature_der = self.signed_ledger_snapshot(&mut ledger)?;
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let _commit = cancellation
            .begin_commit()
            .ok_or(VoiceAuthorizationError::OwnerCancelled)?;
        ledger.persist(&signature_der)?;
        Ok(lease)
    }

    fn begin_worker_authorization_session(
        &self,
        cancellation: &CancelToken,
        deadline: Instant,
        needs: WorkerAuthorizationNeeds,
    ) -> Result<Arc<AuthorizedWorkerSession>, VoiceAuthorizationError> {
        let now = Instant::now();
        if cancellation.is_cancelled() {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        validate_worker_session_request(now, deadline, needs)?;
        let catalog = self.catalog()?;
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let owner = self.owner_payload_from_ledger(&catalog, &mut ledger)?;
        let state_serial = ledger.value.serial;
        let mut assets = Vec::with_capacity(2);
        for (required, role, purpose) in [
            (needs.asr, WorkerRole::Asr, AssetPurpose::AsrWeights),
            (needs.llm, WorkerRole::Llm, AssetPurpose::LlmWeights),
        ] {
            if !required {
                continue;
            }
            let frozen = catalog.asset_by_purpose(purpose)?;
            let receipt = ledger
                .value
                .active_assets
                .iter()
                .find(|receipt| {
                    receipt.asset_id == frozen.asset_id
                        && receipt.state == AuthorizedAssetState::Enabled
                })
                .ok_or(VoiceAuthorizationError::AssetMismatch)?;
            if receipt.sha256 != frozen.disclosure.digest_sha256
                || receipt.version != receipt.sha256
                || receipt.bytes != frozen.disclosure.expected_bytes.0
                || receipt.purpose != purpose_name(purpose)
            {
                return Err(VoiceAuthorizationError::AssetMismatch);
            }
            assets.push(SessionAsset {
                role,
                asset_id: receipt.asset_id.clone(),
                sha256: receipt.sha256.clone(),
                generation: receipt.generation,
            });
        }
        drop(ledger);
        for asset in &assets {
            self.approve_worker_closure(asset.role)?;
        }
        if cancellation.is_cancelled() || Instant::now() >= deadline {
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let key_session = self.key.preauthorize(PROMPT_WORKER_SESSION)?;
        if cancellation.is_cancelled() || Instant::now() >= deadline || !key_session.is_valid() {
            key_session.invalidate();
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let public_key = self.key.public_key(PROMPT_BOOTSTRAP)?;
        let session = Arc::new(AuthorizedWorkerSession {
            service: Arc::new(self.clone()),
            key_session,
            valid: AtomicBool::new(true),
            gate: Mutex::new(()),
            expires_at: deadline.min(now + Duration::from_secs(5 * 60)),
            state_serial,
            binding_sha256: catalog.binding_sha256.clone(),
            parent_executable_sha256: owner.parent_executable_sha256.clone(),
            parent_resource_closure_sha256: owner.parent_resource_closure_sha256.clone(),
            owner_public_key_sha256: sha256_hex(&public_key),
            assets,
        });
        if cancellation.is_cancelled() || Instant::now() >= session.expires_at {
            session.invalidate();
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        let snapshot_is_current = session.service.session_snapshot_is_current(&session)?;
        if cancellation.is_cancelled() || Instant::now() >= session.expires_at {
            session.invalidate();
            return Err(VoiceAuthorizationError::OwnerCancelled);
        }
        if !snapshot_is_current {
            session.invalidate();
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        Ok(session)
    }

    fn finish_transfer(
        &self,
        lease: VoiceTransferLease,
        result: TransferResult,
        cancellation: &CancelToken,
    ) -> Result<(), VoiceAuthorizationError> {
        let catalog = self.catalog()?;
        let mut ledger = self.locked(&catalog)?;
        verify_ledger_signatures(self.key.as_ref(), &mut ledger.value)?;
        let record = ledger
            .value
            .pending
            .as_mut()
            .ok_or(VoiceAuthorizationError::GrantInvalid)?;
        if !matches!(
            record.state,
            GrantState::Consuming | GrantState::PurposeBound
        ) || record.payload.generation != lease.generation
            || record.payload.grant_id_hex != encode_lower_hex(&lease.grant_id)
            || record.payload.asset_id != lease.asset_id
            || record.payload.purpose != purpose_name(lease.purpose)
            || record.payload.expected_bytes != lease.expected_bytes
            || record.payload.expected_sha256 != lease.expected_sha256
            || record.payload.binding_sha256 != lease.binding_sha256
            || record.payload.expires_at_unix != lease.expires_at_unix
        {
            return Err(VoiceAuthorizationError::GrantConsumed);
        }
        match result.outcome {
            TransferOutcome::Cancelled => Err(VoiceAuthorizationError::OwnerCancelled),
            TransferOutcome::Failed => {
                if cancellation.is_cancelled() {
                    return Err(VoiceAuthorizationError::OwnerCancelled);
                }
                record.state = GrantState::Failed;
                ledger.value.archive_terminal_pending()?;
                let signature_der = self.signed_ledger_snapshot(&mut ledger)?;
                if cancellation.is_cancelled() {
                    return Err(VoiceAuthorizationError::OwnerCancelled);
                }
                let _commit = cancellation
                    .begin_commit()
                    .ok_or(VoiceAuthorizationError::OwnerCancelled)?;
                ledger.persist(&signature_der)
            }
            TransferOutcome::Promoted => {
                if cancellation.is_cancelled() {
                    return Err(VoiceAuthorizationError::OwnerCancelled);
                }
                if result.bytes_received != lease.expected_bytes
                    || result.sha256 != lease.expected_sha256
                    || result.promoted_version.as_deref() != Some(lease.expected_sha256.as_str())
                {
                    record.state = GrantState::Failed;
                    ledger.value.archive_terminal_pending()?;
                    let signature_der = self.signed_ledger_snapshot(&mut ledger)?;
                    if cancellation.is_cancelled() {
                        return Err(VoiceAuthorizationError::OwnerCancelled);
                    }
                    let _commit = cancellation
                        .begin_commit()
                        .ok_or(VoiceAuthorizationError::OwnerCancelled)?;
                    ledger.persist(&signature_der)?;
                    return Err(VoiceAuthorizationError::AssetMismatch);
                }
                let qwen = lease.purpose == AssetPurpose::LlmWeights;
                let template = result.template_fingerprint_sha256.as_deref();
                let tokenizer = result.tokenizer_metadata_sha256.as_deref();
                if qwen {
                    let proof = record
                        .purpose_proof
                        .as_ref()
                        .ok_or(VoiceAuthorizationError::GrantInvalid)?;
                    if record.state != GrantState::PurposeBound
                        || template != Some(proof.payload.template_sha256.as_str())
                        || tokenizer != Some(proof.payload.tokenizer_metadata_sha256.as_str())
                    {
                        return Err(VoiceAuthorizationError::ModelInvalid);
                    }
                } else if template.is_some()
                    || tokenizer.is_some()
                    || record.purpose_proof.is_some()
                {
                    return Err(VoiceAuthorizationError::ModelInvalid);
                }
                if let Some(existing) = ledger
                    .value
                    .active_assets
                    .iter_mut()
                    .find(|receipt| receipt.asset_id == lease.asset_id)
                {
                    existing.state = AuthorizedAssetState::Revoked;
                }
                let generation = lease.generation;
                ledger.value.active_assets.push(ActiveAssetReceipt {
                    generation,
                    asset_id: lease.asset_id.clone(),
                    purpose: purpose_name(lease.purpose).to_owned(),
                    version: lease.expected_sha256.clone(),
                    bytes: lease.expected_bytes,
                    sha256: lease.expected_sha256.clone(),
                    template_fingerprint_sha256: template.map(str::to_owned),
                    tokenizer_metadata_sha256: tokenizer.map(str::to_owned),
                    state: AuthorizedAssetState::Enabled,
                });
                record.state = GrantState::Promoted;
                record.promoted_version = Some(lease.expected_sha256);
                ledger.value.archive_terminal_pending()?;
                if cancellation.is_cancelled() {
                    return Err(VoiceAuthorizationError::OwnerCancelled);
                }
                let signature_der = self.signed_ledger_snapshot(&mut ledger)?;
                if cancellation.is_cancelled() {
                    return Err(VoiceAuthorizationError::OwnerCancelled);
                }
                let _commit = cancellation
                    .begin_commit()
                    .ok_or(VoiceAuthorizationError::OwnerCancelled)?;
                ledger.persist(&signature_der)
            }
        }
    }
}

fn verify_bundled_binding_resource(resource_dir: &Path) -> Result<(), VoiceAuthorizationError> {
    let expected = super::catalog::EMBEDDED_BINDING;
    let path = resource_dir.join("saturday-resources/voice-asset-bindings.json");
    let metadata =
        std::fs::symlink_metadata(&path).map_err(|_| VoiceAuthorizationError::BindingInvalid)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() != expected.len() as u64
    {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    let file = File::open(path).map_err(|_| VoiceAuthorizationError::BindingInvalid)?;
    let bytes = read_bounded(&file, expected.len())?;
    if bytes != expected {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    Ok(())
}

fn validate_live_caller(
    app: &tauri::AppHandle,
    caller: &NativeCallerContext,
) -> Result<(), VoiceAuthorizationError> {
    if !caller.is_trusted() {
        return Err(VoiceAuthorizationError::ProtocolFailure);
    }
    let window = app
        .get_webview_window(caller.window_label())
        .ok_or(VoiceAuthorizationError::ProtocolFailure)?;
    caller
        .ensure_live(&window)
        .map_err(|_| VoiceAuthorizationError::ProtocolFailure)
}

fn validate_catalog_identity(
    catalog: &FrozenCatalog,
    app_identity: &crate::platform::voice_authorization::CodeIdentityDigest,
) -> Result<(), VoiceAuthorizationError> {
    for asset in &catalog.assets {
        if asset.code_identity.application_team_id_sha256 != app_identity.team_id_sha256
            || asset.code_identity.application_requirement_sha256
                != app_identity.designated_requirement_sha256
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
        if asset.code_identity.worker_team_id_sha256
            != asset.code_identity.application_team_id_sha256
        {
            return Err(VoiceAuthorizationError::WorkerMismatch);
        }
    }
    Ok(())
}

struct GrantRequest<'a> {
    asset: &'a FrozenAsset,
    catalog: &'a FrozenCatalog,
    generation: u64,
    grant_id: &'a [u8; 16],
    issued_at: u64,
    expires_at: u64,
    nonce: &'a [u8; 32],
    owner: &'a OwnerBindingPayload,
    use_expires_at: u64,
}

fn grant_payload(request: GrantRequest<'_>) -> Result<GrantPayload, VoiceAuthorizationError> {
    let GrantRequest {
        asset,
        catalog,
        generation,
        grant_id,
        issued_at,
        expires_at,
        nonce,
        owner,
        use_expires_at,
    } = request;
    let payload = GrantPayload {
        schema: VOICE_GRANT_SCHEMA.to_owned(),
        request_role: "voice_asset_install".to_owned(),
        generation,
        grant_id_hex: encode_lower_hex(grant_id),
        asset_id: asset.asset_id.clone(),
        purpose: purpose_name(asset.purpose).to_owned(),
        worker_role: asset.worker.role,
        expected_bytes: asset.disclosure.expected_bytes.0,
        expected_sha256: asset.disclosure.digest_sha256.clone(),
        binding_sha256: catalog.binding_sha256.clone(),
        source_generation_sha256: asset.source_generation_sha256.clone(),
        source_tree_sha256: asset.source_tree_sha256.clone(),
        resource_generation_sha256: asset.resource_generation_sha256.clone(),
        worker_sha256: asset.worker.sha256.clone(),
        worker_bytes: asset.worker.bytes,
        worker_source_seal_sha256: asset.worker.source_seal_sha256.clone(),
        worker_resource_seal_sha256: asset.worker.resource_seal_sha256.clone(),
        template_fingerprint_sha256: asset.template_fingerprint_sha256.clone(),
        application_bundle_identifier: owner.application_bundle_identifier.clone(),
        application_team_id_sha256: owner.application_team_id_sha256.clone(),
        application_requirement_sha256: owner.application_requirement_sha256.clone(),
        parent_executable_sha256: owner.parent_executable_sha256.clone(),
        parent_resource_closure_sha256: owner.parent_resource_closure_sha256.clone(),
        worker_team_id_sha256: asset.code_identity.worker_team_id_sha256.clone(),
        worker_requirement_sha256: asset.code_identity.worker_requirement_sha256.clone(),
        issued_at_unix: issued_at,
        expires_at_unix: expires_at,
        use_expires_at_unix: use_expires_at,
        consent_nonce_sha256: sha256_hex(nonce),
    };
    payload.validate(&catalog.binding_sha256, generation)?;
    Ok(payload)
}

fn verify_ledger_signatures(
    key: &dyn VoiceOwnerKey,
    ledger: &mut DurableLedger,
) -> Result<(), VoiceAuthorizationError> {
    let Some(public_key_hex) = ledger.owner_public_key_x963.as_deref() else {
        if ledger.serial == 0
            && ledger.pending.is_none()
            && ledger.history.is_empty()
            && ledger.active_assets.is_empty()
        {
            return Ok(());
        }
        return Err(VoiceAuthorizationError::KeyUnavailable);
    };
    let public_key = decode_lower_hex(public_key_hex)?;
    if key.public_key("Verify the local Saturday voice authorization key")? != public_key {
        return Err(VoiceAuthorizationError::KeyUnavailable);
    }
    for record in ledger.history.iter().chain(ledger.pending.iter()) {
        let grant_bytes = signed_bytes(VOICE_GRANT_DOMAIN, &record.payload)?;
        if !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(&public_key, &grant_bytes, &record.signature_der)? {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        if let Some(purpose) = &record.purpose_proof {
            let purpose_bytes = signed_bytes(VOICE_MODEL_PURPOSE_DOMAIN, &purpose.payload)?;
            if !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(&public_key, &purpose_bytes, &purpose.signature_der)? {
                return Err(VoiceAuthorizationError::StorageInvalid);
            }
        }
    }
    let _ = key;
    Ok(())
}

fn verify_proof(
    key: &dyn VoiceOwnerKey,
    ledger: &DurableLedger,
    message: &[u8],
    proof: &VoiceKeyProof,
) -> Result<(), VoiceAuthorizationError> {
    if let Some(existing) = ledger.owner_public_key_x963.as_deref() {
        if decode_lower_hex(existing)? != proof.public_key_x963 {
            return Err(VoiceAuthorizationError::KeyUnavailable);
        }
    }
    if !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(&proof.public_key_x963, message, &proof.signature_der)? {
        return Err(VoiceAuthorizationError::OwnerAuthentication);
    }
    let _ = key;
    Ok(())
}

fn verify_bootstrap_record_for_identity(
    bytes: &[u8],
    identity: &crate::platform::voice_authorization::CodeIdentityDigest,
) -> Result<(), VoiceAuthorizationError> {
    let signed: ParentSigned<OwnerKeyBootstrapPayload> =
        serde_json::from_slice(bytes).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
    if canonical_parent_json(&signed)? != bytes
        || signed.payload.application_bundle_identifier != identity.bundle_identifier
        || signed.payload.application_team_id_sha256 != identity.team_id_sha256
        || signed.payload.application_requirement_sha256 != identity.designated_requirement_sha256
        || signed.payload.application_tag_sha256 != sha256_hex(VOICE_KEY_APPLICATION_TAG)
        || signed.payload.owner_public_key_sha256
            != sha256_hex(&signed.payload.owner_public_key_x963)
        || signed.payload.issued_at_unix > current_unix_seconds()?
        || parent_auth::verify_owner_key_bootstrap_record(bytes).map_err(map_parent_auth_error)?
            != signed.payload.owner_public_key_x963
    {
        return Err(VoiceAuthorizationError::OwnerAuthentication);
    }
    Ok(())
}

fn signed_bytes<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Vec<u8>, VoiceAuthorizationError> {
    let encoded =
        serde_json::to_vec(value).map_err(|_| VoiceAuthorizationError::ProtocolFailure)?;
    if encoded.is_empty() || encoded.len() > MAX_SIGNED_RECORD_BYTES {
        return Err(VoiceAuthorizationError::ProtocolFailure);
    }
    let mut bytes = Vec::with_capacity(domain.len() + encoded.len());
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&encoded);
    Ok(bytes)
}

fn nonce_already_used(ledger: &DurableLedger, nonce_sha256: &str) -> bool {
    ledger.last_nonce_sha256.as_deref() == Some(nonce_sha256)
        || ledger
            .history
            .iter()
            .chain(ledger.pending.iter())
            .any(|record| {
                record.payload.consent_nonce_sha256.as_str() == nonce_sha256
                    || record.purpose_proof.as_ref().is_some_and(|proof| {
                        proof.payload.consent_nonce_sha256.as_str() == nonce_sha256
                    })
            })
}

fn canonical_parent_json<T: Serialize>(value: &T) -> Result<Vec<u8>, VoiceAuthorizationError> {
    let value = serde_json::to_value(value).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
    serde_json::to_vec(&value).map_err(|_| VoiceAuthorizationError::StorageInvalid)
}

fn owner_static_bytes(owner: &OwnerBindingPayload) -> Result<Vec<u8>, VoiceAuthorizationError> {
    let mut owner = owner.clone();
    owner.owner_public_key_x963.clear();
    owner.owner_public_key_sha256.clear();
    owner.issued_at_unix = 0;
    owner.expires_at_unix = 0;
    owner.nonce_sha256.clear();
    serde_json::to_vec(&owner).map_err(|_| VoiceAuthorizationError::StorageInvalid)
}

fn map_parent_auth_error(error: parent_auth::ParentAuthError) -> VoiceAuthorizationError {
    match error {
        parent_auth::ParentAuthError::SourceReviewRequired => {
            VoiceAuthorizationError::BindingUnfrozen
        }
        parent_auth::ParentAuthError::PlatformUnavailable => {
            VoiceAuthorizationError::KeyUnavailable
        }
        parent_auth::ParentAuthError::Expired => VoiceAuthorizationError::GrantExpired,
        parent_auth::ParentAuthError::InvalidSignature => {
            VoiceAuthorizationError::OwnerAuthentication
        }
        parent_auth::ParentAuthError::ParentIdentityMismatch
        | parent_auth::ParentAuthError::WorkerIdentityMismatch
        | parent_auth::ParentAuthError::ResourceClosureMismatch => {
            VoiceAuthorizationError::WorkerMismatch
        }
        parent_auth::ParentAuthError::InvalidGrant => VoiceAuthorizationError::GrantInvalid,
    }
}

fn owner_setup_copy(owner: &OwnerBindingPayload, catalog: &FrozenCatalog) -> (String, String) {
    let asr = catalog
        .assets
        .iter()
        .find(|asset| asset.purpose == AssetPurpose::AsrWeights);
    let llm = catalog
        .assets
        .iter()
        .find(|asset| asset.purpose == AssetPurpose::LlmWeights);
    let asset_details = |asset: Option<&FrozenAsset>, label: &str| {
        asset.map_or_else(
            || format!("{label}: unavailable"),
            |asset| {
                format!(
                    "{label}: {} bytes; model SHA-256 {}; pinned source {}; worker SHA-256 {}",
                    asset.disclosure.expected_bytes.0,
                    asset.disclosure.digest_sha256,
                    asset.disclosure.source_revision,
                    asset.worker.sha256,
                )
            },
        )
    };
    let english = format!(
        "Create the dedicated hardware Secure Enclave key for Saturday local voice. This binds the independently reviewed asset-binding SHA-256 {}, reviewer-roster SHA-256 {}, signed app identifier {}, team-ID SHA-256 {}, designated requirement SHA-256 {}, post-sign executable SHA-256 {}, and complete packaged voice-resource closure SHA-256 {}.\n{}\n{}\nCancel creates no key or owner binding. Continue stores a signed owner anchor only in this app's private data directory; it never edits the signed application bundle. Touch ID is required.",
        owner.binding_sha256,
        owner.reviewer_roster_sha256,
        owner.application_bundle_identifier,
        owner.application_team_id_sha256,
        owner.application_requirement_sha256,
        owner.parent_executable_sha256,
        owner.parent_resource_closure_sha256,
        asset_details(asr, "Whisper ASR"),
        asset_details(llm, "Qwen local inference"),
    );
    let turkish = format!(
        "Saturday yerel ses özelliği için bu cihaza özel Secure Enclave donanım anahtarı oluşturulur. Anahtar; bağımsız olarak incelenmiş varlık bağlama SHA-256 değeri {}, inceleyenler listesi SHA-256 değeri {}, imzalı uygulama kimliği {}, takım kimliği SHA-256 değeri {}, belirlenmiş imza gereksinimi SHA-256 değeri {}, imzalama sonrası çalıştırılabilir dosya SHA-256 değeri {} ve paketlenmiş tüm ses kaynaklarının SHA-256 değeri {} ile bağlanır.\n{}\n{}\nVazgeçerseniz anahtar veya sahip bağlaması oluşturulmaz. Devam ederseniz imzalı sahip kaydı yalnızca bu uygulamanın özel veri dizinine yazılır; imzalı uygulama paketi değiştirilmez. Touch ID gerekir.",
        owner.binding_sha256,
        owner.reviewer_roster_sha256,
        owner.application_bundle_identifier,
        owner.application_team_id_sha256,
        owner.application_requirement_sha256,
        owner.parent_executable_sha256,
        owner.parent_resource_closure_sha256,
        asset_details(asr, "Whisper ASR"),
        asset_details(llm, "Qwen yerel çıkarım"),
    );
    (english, turkish)
}

fn validate_verified_model_file(
    candidate: &VerifiedModelFile,
    asset: &FrozenAsset,
) -> Result<(), VoiceAuthorizationError> {
    let metadata = candidate
        .file()
        .metadata()
        .map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
    // SAFETY: F_GETFL only inspects a valid descriptor's access mode.
    let flags = unsafe { libc::fcntl(candidate.file().as_raw_fd(), libc::F_GETFL) };
    if !metadata.is_file()
        || metadata.len() != candidate.bytes()
        || metadata.dev() != candidate.device
        || metadata.ino() != candidate.inode
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() } as u32
        || metadata.mode() & 0o777 != 0o400
        || flags < 0
        || flags & libc::O_ACCMODE != libc::O_RDONLY
        || candidate.bytes() != asset.disclosure.expected_bytes.0
        || candidate.payload_sha256() != asset.disclosure.digest_sha256
        || hash_file(candidate.file())? != candidate.payload_sha256()
    {
        return Err(VoiceAuthorizationError::ModelInvalid);
    }
    Ok(())
}

fn model_payload_relative_path(asset: &FrozenAsset) -> PathBuf {
    let role = match asset.purpose {
        AssetPurpose::AsrWeights => "asr",
        AssetPurpose::LlmWeights => "llm",
        AssetPurpose::Runtime => "runtime",
        AssetPurpose::PortableVoice => "portable_voice",
    };
    PathBuf::from("voice/assets/models")
        .join(role)
        .join(&asset.asset_id)
        .join("versions")
        .join(&asset.disclosure.digest_sha256)
        .join("payload")
}

fn model_sandbox_path(asset: &FrozenAsset) -> PathBuf {
    PathBuf::from("/voice-models")
        .join(match asset.purpose {
            AssetPurpose::AsrWeights => "asr",
            AssetPurpose::LlmWeights => "llm",
            AssetPurpose::Runtime => "runtime",
            AssetPurpose::PortableVoice => "portable_voice",
        })
        .join(&asset.asset_id)
        .join(&asset.disclosure.digest_sha256)
        .join("payload")
}

fn purpose_name(purpose: AssetPurpose) -> &'static str {
    match purpose {
        AssetPurpose::AsrWeights => "asr_weights",
        AssetPurpose::LlmWeights => "llm_weights",
        AssetPurpose::Runtime => "runtime",
        AssetPurpose::PortableVoice => "portable_voice",
    }
}

fn parse_purpose(value: &str) -> Result<AssetPurpose, VoiceAuthorizationError> {
    match value {
        "asr_weights" => Ok(AssetPurpose::AsrWeights),
        "llm_weights" => Ok(AssetPurpose::LlmWeights),
        _ => Err(VoiceAuthorizationError::StorageInvalid),
    }
}

fn hash_file(file: &File) -> Result<String, VoiceAuthorizationError> {
    let metadata = file
        .metadata()
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    if !metadata.is_file() {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    let mut digest = Sha256::new();
    let mut offset = 0u64;
    let mut buffer = [0u8; 1024 * 1024];
    while offset < metadata.len() {
        let length = usize::try_from((metadata.len() - offset).min(buffer.len() as u64))
            .map_err(|_| VoiceAuthorizationError::ModelInvalid)?;
        let mut read = 0;
        while read < length {
            let count = file
                .read_at(&mut buffer[read..length], offset + read as u64)
                .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
            if count == 0 {
                return Err(VoiceAuthorizationError::ModelInvalid);
            }
            read += count;
        }
        digest.update(&buffer[..length]);
        offset += length as u64;
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn read_bounded(file: &File, maximum: usize) -> Result<Vec<u8>, VoiceAuthorizationError> {
    let length = usize::try_from(
        file.metadata()
            .map_err(|_| VoiceAuthorizationError::StorageFailure)?
            .len(),
    )
    .map_err(|_| VoiceAuthorizationError::BindingInvalid)?;
    if length == 0 || length > maximum {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    let mut bytes = Vec::with_capacity(length);
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    if bytes.len() != length {
        return Err(VoiceAuthorizationError::BindingInvalid);
    }
    Ok(bytes)
}

#[cfg(test)]
mod worker_session_request_tests {
    use super::{
        session_state_serial_is_current, validate_worker_session_request, WorkerAuthorizationNeeds,
    };
    use crate::voice_authorization::VoiceAuthorizationError;
    use std::time::{Duration, Instant};

    #[test]
    fn rejects_empty_role_needs_before_session_preparation() {
        let now = Instant::now();
        assert_eq!(
            validate_worker_session_request(
                now,
                now + Duration::from_secs(60),
                WorkerAuthorizationNeeds {
                    asr: false,
                    llm: false
                },
            ),
            Err(VoiceAuthorizationError::ProtocolFailure),
        );
    }

    #[test]
    fn accepts_only_needed_roles_within_five_minute_lifetime() {
        let now = Instant::now();
        assert!(validate_worker_session_request(
            now,
            now + Duration::from_secs(5 * 60),
            WorkerAuthorizationNeeds {
                asr: true,
                llm: false
            },
        )
        .is_ok());
        assert_eq!(
            validate_worker_session_request(
                now,
                now + Duration::from_secs(5 * 60 + 1),
                WorkerAuthorizationNeeds {
                    asr: false,
                    llm: true
                },
            ),
            Err(VoiceAuthorizationError::ProtocolFailure),
        );
    }

    #[test]
    fn worker_session_state_change_invalidates_authorization_snapshot() {
        assert!(session_state_serial_is_current(41, 41));
        assert!(!session_state_serial_is_current(41, 42));
    }
}
