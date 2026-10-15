use super::{
    is_sha256, VoiceAuthorizationError, VoiceOwnerKey, VOICE_GRANT_SCHEMA, VOICE_LEDGER_SCHEMA,
    VOICE_TRANSFER_TTL_SECONDS,
};
use saturday_voice_parent_auth::{
    self as parent_auth, AuthorizationAssetState, AuthorizationStatePayload,
    SignedAuthorizationState,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

pub(super) const MAX_LEDGER_BYTES: usize = 65_536;
const PRIVATE_DIR_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;
const READ_ONLY_MODEL_MODE: u32 = 0o400;
const SIGNED_LEDGER_ENVELOPE_SCHEMA: &str = "saturday.voice.authorization-ledger-envelope.v2";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DurableLedger {
    pub(super) schema: String,
    pub(super) binding_sha256: String,
    pub(super) serial: u64,
    pub(super) owner_public_key_x963: Option<String>,
    pub(super) last_nonce_sha256: Option<String>,
    pub(super) pending: Option<GrantRecord>,
    pub(super) history: Vec<GrantRecord>,
    pub(super) active_assets: Vec<ActiveAssetReceipt>,
}

impl DurableLedger {
    pub(super) fn empty(binding_sha256: String) -> Self {
        Self {
            schema: VOICE_LEDGER_SCHEMA.to_owned(),
            binding_sha256,
            serial: 0,
            owner_public_key_x963: None,
            last_nonce_sha256: None,
            pending: None,
            history: Vec::new(),
            active_assets: Vec::new(),
        }
    }

    pub(super) fn validate(&self, binding_sha256: &str) -> Result<(), VoiceAuthorizationError> {
        if self.schema != VOICE_LEDGER_SCHEMA
            || self.binding_sha256 != binding_sha256
            || (self.serial == 0
                && (self.pending.is_some()
                    || !self.history.is_empty()
                    || !self.active_assets.is_empty()))
            || self.history.len() > 16
            || self.active_assets.len() > 2
            || (self.serial != 0 && self.owner_public_key_x963.is_none())
            || self.owner_public_key_x963.as_deref().is_some_and(|key| {
                !decode_lower_hex(key).is_ok_and(|key| key.len() == 65 && key[0] == 0x04)
            })
            || self
                .last_nonce_sha256
                .as_deref()
                .is_some_and(|digest| !is_sha256(digest))
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        if let Some(record) = &self.pending {
            record.validate(binding_sha256, self.serial)?;
            let pending_nonce = record
                .purpose_proof
                .as_ref()
                .map(|proof| proof.payload.consent_nonce_sha256.as_str())
                .unwrap_or(record.payload.consent_nonce_sha256.as_str());
            if self.last_nonce_sha256.as_deref() != Some(pending_nonce) {
                return Err(VoiceAuthorizationError::StorageInvalid);
            }
        } else if self.serial != 0 && self.last_nonce_sha256.is_none() {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        let mut grant_ids = HashSet::new();
        let mut nonces = HashSet::new();
        if self
            .history
            .iter()
            .any(|record| !record.state.is_terminal())
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        for record in self.history.iter().chain(self.pending.iter()) {
            record.validate(binding_sha256, self.serial)?;
            if !grant_ids.insert(record.payload.grant_id_hex.as_str())
                || !nonces.insert(record.payload.consent_nonce_sha256.as_str())
            {
                return Err(VoiceAuthorizationError::StorageInvalid);
            }
            if let Some(proof) = &record.purpose_proof {
                if !nonces.insert(proof.payload.consent_nonce_sha256.as_str()) {
                    return Err(VoiceAuthorizationError::StorageInvalid);
                }
            }
        }
        let mut active_ids = HashSet::new();
        for receipt in &self.active_assets {
            receipt.validate(self.serial)?;
            if !active_ids.insert(receipt.asset_id.as_str()) {
                return Err(VoiceAuthorizationError::StorageInvalid);
            }
        }
        Ok(())
    }

    pub(super) fn archive_terminal_pending(&mut self) -> Result<(), VoiceAuthorizationError> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|record| record.state.is_terminal())
        {
            return Ok(());
        }
        let record = self
            .pending
            .take()
            .ok_or(VoiceAuthorizationError::StorageInvalid)?;
        self.history.push(record);
        if self.history.len() > 16 {
            self.history.remove(0);
        }
        Ok(())
    }
}

impl DurableLedger {
    pub(super) fn authorization_state_payload(
        &self,
    ) -> Result<AuthorizationStatePayload, VoiceAuthorizationError> {
        let encoded =
            serde_json::to_vec(self).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
        let mut active_assets = self
            .active_assets
            .iter()
            .map(|receipt| AuthorizationAssetState {
                asset_id: receipt.asset_id.clone(),
                purpose: receipt.purpose.clone(),
                generation: receipt.generation,
                version: receipt.version.clone(),
                bytes: receipt.bytes,
                sha256: receipt.sha256.clone(),
                state: match &receipt.state {
                    super::AuthorizedAssetState::Enabled => "enabled",
                    super::AuthorizedAssetState::Disabled => "disabled",
                    super::AuthorizedAssetState::Revoked => "revoked",
                }
                .to_owned(),
                template_fingerprint_sha256: receipt.template_fingerprint_sha256.clone(),
                tokenizer_metadata_sha256: receipt.tokenizer_metadata_sha256.clone(),
            })
            .collect::<Vec<_>>();
        active_assets.sort_by(|left, right| left.asset_id.cmp(&right.asset_id));
        let payload = AuthorizationStatePayload {
            schema: parent_auth::AUTHORIZATION_STATE_SCHEMA.to_owned(),
            binding_sha256: self.binding_sha256.clone(),
            serial: self.serial,
            ledger_sha256: super::sha256_hex(&encoded),
            active_assets,
        };
        payload
            .validate()
            .map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
        Ok(payload)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GrantRecord {
    pub(super) payload: GrantPayload,
    pub(super) signature_der: Vec<u8>,
    pub(super) state: GrantState,
    pub(super) purpose_proof: Option<PurposeRecord>,
    pub(super) promoted_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GrantState {
    Issued,
    Consuming,
    PurposeBound,
    Promoted,
    Cancelled,
    Failed,
    Interrupted,
}

impl GrantState {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Promoted | Self::Cancelled | Self::Failed | Self::Interrupted
        )
    }
}

impl GrantRecord {
    fn validate(&self, binding_sha256: &str, serial: u64) -> Result<(), VoiceAuthorizationError> {
        self.payload.validate(binding_sha256, serial)?;
        if self.signature_der.len() < 8
            || self.signature_der.len() > 256
            || self.signature_der[0] != 0x30
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        let qwen = self.payload.purpose == "llm_weights";
        match &self.state {
            GrantState::Issued | GrantState::Consuming => {
                if self.purpose_proof.is_some() || self.promoted_version.is_some() {
                    return Err(VoiceAuthorizationError::StorageInvalid);
                }
            }
            GrantState::PurposeBound => {
                if !qwen || self.promoted_version.is_some() {
                    return Err(VoiceAuthorizationError::StorageInvalid);
                }
                self.purpose_proof
                    .as_ref()
                    .ok_or(VoiceAuthorizationError::StorageInvalid)?
                    .validate(&self.payload, serial)?;
            }
            GrantState::Promoted => {
                if self.promoted_version.as_deref() != Some(self.payload.expected_sha256.as_str()) {
                    return Err(VoiceAuthorizationError::StorageInvalid);
                }
                if qwen {
                    self.purpose_proof
                        .as_ref()
                        .ok_or(VoiceAuthorizationError::StorageInvalid)?
                        .validate(&self.payload, serial)?;
                } else if self.purpose_proof.is_some() {
                    return Err(VoiceAuthorizationError::StorageInvalid);
                }
            }
            GrantState::Cancelled | GrantState::Failed | GrantState::Interrupted => {
                if self.promoted_version.is_some() {
                    return Err(VoiceAuthorizationError::StorageInvalid);
                }
                if let Some(proof) = &self.purpose_proof {
                    proof.validate(&self.payload, serial)?;
                }
            }
        }
        Ok(())
    }
}

impl GrantPayload {
    pub(super) fn validate(
        &self,
        binding_sha256: &str,
        serial: u64,
    ) -> Result<(), VoiceAuthorizationError> {
        let (valid_asset, expected_bytes, expected_sha256, expected_role) =
            match self.asset_id.as_str() {
                "whisper-base" if self.purpose == "asr_weights" => (
                    true,
                    147_951_465,
                    "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
                    super::WorkerRole::Asr,
                ),
                "qwen3-4b-q4-k-m" if self.purpose == "llm_weights" => (
                    true,
                    2_497_280_256,
                    "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5",
                    super::WorkerRole::Llm,
                ),
                _ => (false, 0, "", super::WorkerRole::Asr),
            };
        if self.schema != VOICE_GRANT_SCHEMA
            || self.request_role != "voice_asset_install"
            || self.generation == 0
            || self.generation > serial
            || self.grant_id_hex.len() != 32
            || !decode_lower_hex(&self.grant_id_hex).is_ok_and(|bytes| bytes.len() == 16)
            || !valid_asset
            || self.worker_role != expected_role
            || self.expected_bytes != expected_bytes
            || self.expected_sha256 != expected_sha256
            || !is_sha256(&self.binding_sha256)
            || self.binding_sha256 != binding_sha256
            || !is_sha256(&self.source_generation_sha256)
            || !is_sha256(&self.source_tree_sha256)
            || !is_sha256(&self.resource_generation_sha256)
            || !is_sha256(&self.worker_sha256)
            || self.worker_bytes == 0
            || !is_sha256(&self.worker_source_seal_sha256)
            || !is_sha256(&self.worker_resource_seal_sha256)
            || self
                .template_fingerprint_sha256
                .as_deref()
                .is_some_and(|hash| !is_sha256(hash))
            || !valid_bundle_id(&self.application_bundle_identifier)
            || !is_sha256(&self.application_team_id_sha256)
            || !is_sha256(&self.application_requirement_sha256)
            || !is_sha256(&self.parent_executable_sha256)
            || !is_sha256(&self.parent_resource_closure_sha256)
            || !is_sha256(&self.worker_team_id_sha256)
            || !is_sha256(&self.worker_requirement_sha256)
            || self.issued_at_unix == 0
            || self.expires_at_unix <= self.issued_at_unix
            || self.expires_at_unix - self.issued_at_unix > VOICE_TRANSFER_TTL_SECONDS
            || self.use_expires_at_unix <= self.issued_at_unix
            || self.use_expires_at_unix - self.issued_at_unix
                > super::VOICE_PARENT_GRANT_TTL_SECONDS
            || !is_sha256(&self.consent_nonce_sha256)
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        Ok(())
    }
}

impl PurposeRecord {
    fn validate(&self, grant: &GrantPayload, serial: u64) -> Result<(), VoiceAuthorizationError> {
        let payload = &self.payload;
        if grant.purpose != "llm_weights"
            || self.signature_der.len() < 8
            || self.signature_der.len() > 256
            || self.signature_der[0] != 0x30
            || payload.schema != "saturday.voice.model-purpose.v1"
            || payload.serial <= grant.generation
            || payload.serial > serial
            || payload.asset_id != grant.asset_id
            || payload.model_sha256 != grant.expected_sha256
            || !is_sha256(&payload.model_sha256)
            || !is_sha256(&payload.template_sha256)
            || !is_sha256(&payload.tokenizer_metadata_sha256)
            || payload.architecture != "qwen3"
            || payload.context_length != 32_768
            || payload.binding_sha256 != grant.binding_sha256
            || payload.source_generation_sha256 != grant.source_generation_sha256
            || payload.source_tree_sha256 != grant.source_tree_sha256
            || payload.resource_generation_sha256 != grant.resource_generation_sha256
            || payload.worker_sha256 != grant.worker_sha256
            || payload.worker_source_seal_sha256 != grant.worker_source_seal_sha256
            || payload.worker_resource_seal_sha256 != grant.worker_resource_seal_sha256
            || payload.application_bundle_identifier != grant.application_bundle_identifier
            || !valid_bundle_id(&payload.application_bundle_identifier)
            || payload.application_team_id_sha256 != grant.application_team_id_sha256
            || payload.application_requirement_sha256 != grant.application_requirement_sha256
            || payload.parent_executable_sha256 != grant.parent_executable_sha256
            || payload.parent_resource_closure_sha256 != grant.parent_resource_closure_sha256
            || payload.worker_team_id_sha256 != grant.worker_team_id_sha256
            || payload.worker_requirement_sha256 != grant.worker_requirement_sha256
            || payload.issued_at_unix == 0
            || payload.expires_at_unix <= payload.issued_at_unix
            || payload.expires_at_unix > grant.use_expires_at_unix
            || payload.expires_at_unix - payload.issued_at_unix
                > super::VOICE_PARENT_GRANT_TTL_SECONDS
            || !is_sha256(&payload.consent_nonce_sha256)
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PurposeRecord {
    pub(super) payload: PurposePayload,
    pub(super) signature_der: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PurposePayload {
    pub(super) schema: String,
    pub(super) serial: u64,
    pub(super) asset_id: String,
    pub(super) model_sha256: String,
    pub(super) template_sha256: String,
    pub(super) tokenizer_metadata_sha256: String,
    pub(super) architecture: String,
    pub(super) context_length: u32,
    pub(super) binding_sha256: String,
    pub(super) source_generation_sha256: String,
    pub(super) source_tree_sha256: String,
    pub(super) resource_generation_sha256: String,
    pub(super) worker_sha256: String,
    pub(super) worker_source_seal_sha256: String,
    pub(super) worker_resource_seal_sha256: String,
    pub(super) application_bundle_identifier: String,
    pub(super) application_team_id_sha256: String,
    pub(super) application_requirement_sha256: String,
    pub(super) parent_executable_sha256: String,
    pub(super) parent_resource_closure_sha256: String,
    pub(super) worker_team_id_sha256: String,
    pub(super) worker_requirement_sha256: String,
    pub(super) issued_at_unix: u64,
    pub(super) expires_at_unix: u64,
    pub(super) consent_nonce_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GrantPayload {
    pub(super) schema: String,
    pub(super) request_role: String,
    pub(super) generation: u64,
    pub(super) grant_id_hex: String,
    pub(super) asset_id: String,
    pub(super) purpose: String,
    pub(super) worker_role: super::WorkerRole,
    pub(super) expected_bytes: u64,
    pub(super) expected_sha256: String,
    pub(super) binding_sha256: String,
    pub(super) source_generation_sha256: String,
    pub(super) source_tree_sha256: String,
    pub(super) resource_generation_sha256: String,
    pub(super) worker_sha256: String,
    pub(super) worker_bytes: u64,
    pub(super) worker_source_seal_sha256: String,
    pub(super) worker_resource_seal_sha256: String,
    pub(super) template_fingerprint_sha256: Option<String>,
    pub(super) application_bundle_identifier: String,
    pub(super) application_team_id_sha256: String,
    pub(super) application_requirement_sha256: String,
    pub(super) parent_executable_sha256: String,
    pub(super) parent_resource_closure_sha256: String,
    pub(super) worker_team_id_sha256: String,
    pub(super) worker_requirement_sha256: String,
    pub(super) issued_at_unix: u64,
    pub(super) expires_at_unix: u64,
    pub(super) use_expires_at_unix: u64,
    pub(super) consent_nonce_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActiveAssetReceipt {
    pub(super) generation: u64,
    pub(super) asset_id: String,
    pub(super) purpose: String,
    pub(super) version: String,
    pub(super) bytes: u64,
    pub(super) sha256: String,
    pub(super) template_fingerprint_sha256: Option<String>,
    pub(super) tokenizer_metadata_sha256: Option<String>,
    pub(super) state: super::AuthorizedAssetState,
}

impl ActiveAssetReceipt {
    pub(crate) fn asset_id(&self) -> &str {
        &self.asset_id
    }
    pub(crate) fn purpose(&self) -> &str {
        &self.purpose
    }
    pub(crate) fn version(&self) -> &str {
        &self.version
    }
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }
    pub(crate) fn sha256(&self) -> &str {
        &self.sha256
    }
    pub(crate) fn template_fingerprint_sha256(&self) -> Option<&str> {
        self.template_fingerprint_sha256.as_deref()
    }
    pub(crate) fn tokenizer_metadata_sha256(&self) -> Option<&str> {
        self.tokenizer_metadata_sha256.as_deref()
    }
    pub(crate) fn state(&self) -> super::AuthorizedAssetState {
        self.state
    }

    fn validate(&self, serial: u64) -> Result<(), VoiceAuthorizationError> {
        let llm = self.asset_id == "qwen3-4b-q4-k-m" && self.purpose == "llm_weights";
        let asr = self.asset_id == "whisper-base" && self.purpose == "asr_weights";
        if self.generation == 0
            || self.generation > serial
            || (!llm && !asr)
            || self.version != self.sha256
            || !is_sha256(&self.sha256)
            || self.bytes == 0
            || self
                .template_fingerprint_sha256
                .as_deref()
                .is_some_and(|value| !is_sha256(value))
            || self
                .tokenizer_metadata_sha256
                .as_deref()
                .is_some_and(|value| !is_sha256(value))
            || (llm
                && (self.template_fingerprint_sha256.is_none()
                    || self.tokenizer_metadata_sha256.is_none()))
            || (asr
                && (self.template_fingerprint_sha256.is_some()
                    || self.tokenizer_metadata_sha256.is_some()))
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedLedgerEnvelope {
    schema: String,
    payload: DurableLedger,
    authorization_state: SignedAuthorizationState,
}

pub(super) struct LockedLedger {
    directory: File,
    _lock: File,
    pub(super) value: DurableLedger,
    authorization_state: Option<SignedAuthorizationState>,
}

impl LockedLedger {
    pub(super) fn open(
        app_data_dir: &Path,
        binding_sha256: &str,
    ) -> Result<Self, VoiceAuthorizationError> {
        let directory = open_voice_authorization_dir(app_data_dir)?;
        let lock = open_private_file(&directory, "ledger.lock", libc::O_RDWR | libc::O_CREAT)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(VoiceAuthorizationError::StorageFailure);
        }
        cleanup_temporary_files(&directory)?;
        let protected_state =
            crate::platform::voice_authorization::read_protected_authorization_state()
                .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
        let (value, authorization_state) = match read_ledger(&directory, binding_sha256)? {
            Some(envelope) => {
                if !protected_state_matches(
                    &envelope.authorization_state,
                    protected_state.as_deref(),
                )? || envelope.authorization_state.payload
                    != envelope.payload.authorization_state_payload()?
                {
                    return Err(VoiceAuthorizationError::StorageInvalid);
                }
                (envelope.payload, Some(envelope.authorization_state))
            }
            None => {
                if protected_state.is_some() {
                    return Err(VoiceAuthorizationError::StorageInvalid);
                }
                (DurableLedger::empty(binding_sha256.to_owned()), None)
            }
        };
        value.validate(binding_sha256)?;
        Ok(Self {
            directory,
            _lock: lock,
            value,
            authorization_state,
        })
    }
    pub(super) fn open_bootstrap(app_data_dir: &Path) -> Result<Self, VoiceAuthorizationError> {
        let directory = open_voice_authorization_dir(app_data_dir)?;
        let lock = open_private_file(&directory, "ledger.lock", libc::O_RDWR | libc::O_CREAT)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(VoiceAuthorizationError::StorageFailure);
        }
        cleanup_temporary_files(&directory)?;
        Ok(Self {
            directory,
            _lock: lock,
            value: DurableLedger::empty("0".repeat(64)),
            authorization_state: None,
        })
    }

    pub(super) fn read_bootstrap_record(&self) -> Result<Option<Vec<u8>>, VoiceAuthorizationError> {
        read_private_named(
            &self.directory,
            "owner-key-bootstrap.json",
            MAX_LEDGER_BYTES,
        )
    }

    pub(super) fn persist_bootstrap_record(
        &self,
        bytes: &[u8],
    ) -> Result<(), VoiceAuthorizationError> {
        if bytes.is_empty()
            || bytes.len() > MAX_LEDGER_BYTES
            || self.read_bootstrap_record()?.is_some()
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        atomic_write_named(&self.directory, "owner-key-bootstrap.json", bytes)
    }

    pub(super) fn read_bootstrap_witness(
        &self,
    ) -> Result<Option<Vec<u8>>, VoiceAuthorizationError> {
        read_private_named(&self.directory, "owner-key-bootstrap-witness.json", 8192)
    }

    pub(super) fn persist_bootstrap_witness(
        &self,
        bytes: &[u8],
    ) -> Result<(), VoiceAuthorizationError> {
        if bytes.is_empty() || bytes.len() > 8192 {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        atomic_write_named(&self.directory, "owner-key-bootstrap-witness.json", bytes)
    }

    pub(super) fn verify_snapshot(&self, public_key: &[u8]) -> Result<(), VoiceAuthorizationError> {
        let Some(state) = self.authorization_state.as_ref() else {
            let protected_state =
                crate::platform::voice_authorization::read_protected_authorization_state()
                    .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
            if protected_state.is_none()
                && self.value.serial == 0
                && self.value.pending.is_none()
                && self.value.history.is_empty()
                && self.value.active_assets.is_empty()
            {
                return Ok(());
            }
            return Err(VoiceAuthorizationError::StorageInvalid);
        };
        if state.signature_der.len() < 8
            || state.signature_der.len() > 256
            || state.signature_der[0] != 0x30
            || state.payload != self.value.authorization_state_payload()?
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        let protected_state =
            crate::platform::voice_authorization::read_protected_authorization_state()
                .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
        if !protected_state_matches(state, protected_state.as_deref())? {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        let encoded = serde_json::to_vec(&state.payload)
            .map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
        let mut message =
            Vec::with_capacity(parent_auth::AUTHORIZATION_STATE_DOMAIN.len() + encoded.len());
        message.extend_from_slice(parent_auth::AUTHORIZATION_STATE_DOMAIN);
        message.extend_from_slice(&encoded);
        if !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(public_key, &message, &state.signature_der)? {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        Ok(())
    }

    pub(super) fn has_snapshot_signature(&self) -> bool {
        self.authorization_state.is_some()
    }

    pub(super) fn authorization_state(
        &self,
    ) -> Result<&SignedAuthorizationState, VoiceAuthorizationError> {
        self.authorization_state
            .as_ref()
            .ok_or(VoiceAuthorizationError::StorageInvalid)
    }

    pub(super) fn persist(
        &self,
        authorization_state: &SignedAuthorizationState,
    ) -> Result<(), VoiceAuthorizationError> {
        self.value.validate(&self.value.binding_sha256)?;
        if authorization_state.payload != self.value.authorization_state_payload()?
            || authorization_state.signature_der.len() < 8
            || authorization_state.signature_der.len() > 256
            || authorization_state.signature_der.first() != Some(&0x30)
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        let public_key = self
            .value
            .owner_public_key_x963
            .as_deref()
            .ok_or(VoiceAuthorizationError::KeyUnavailable)
            .and_then(decode_lower_hex)?;
        let encoded_state = serde_json::to_vec(&authorization_state.payload)
            .map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
        let mut message =
            Vec::with_capacity(parent_auth::AUTHORIZATION_STATE_DOMAIN.len() + encoded_state.len());
        message.extend_from_slice(parent_auth::AUTHORIZATION_STATE_DOMAIN);
        message.extend_from_slice(&encoded_state);
        if !<crate::platform::voice_authorization::SecurityFrameworkVoiceOwnerKey as VoiceOwnerKey>::verify(&public_key, &message, &authorization_state.signature_der)? {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        let protected_state = canonical_authorization_state(authorization_state)?;
        crate::platform::voice_authorization::store_protected_authorization_state(
            &protected_state,
            authorization_state.payload.serial,
        )
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
        let envelope = SignedLedgerEnvelope {
            schema: SIGNED_LEDGER_ENVELOPE_SCHEMA.to_owned(),
            payload: self.value.clone(),
            authorization_state: authorization_state.clone(),
        };
        let value =
            serde_json::to_value(&envelope).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
        let bytes =
            serde_json::to_vec(&value).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
        if bytes.is_empty() || bytes.len() > MAX_LEDGER_BYTES {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        atomic_write(&self.directory, &bytes)
    }

    pub(super) fn read_owner_anchor(&self) -> Result<Option<Vec<u8>>, VoiceAuthorizationError> {
        let name = c_string("owner-anchor.json")?;
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::ENOENT) => Ok(None),
                _ => Err(VoiceAuthorizationError::StorageFailure),
            };
        }
        let file = unsafe { File::from_raw_fd(fd) };
        validate_regular_file(&file, PRIVATE_FILE_MODE)?;
        let length = usize::try_from(
            file.metadata()
                .map_err(|_| VoiceAuthorizationError::StorageFailure)?
                .len(),
        )
        .map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
        if length == 0 || length > MAX_LEDGER_BYTES {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        let mut bytes = Vec::with_capacity(length);
        (&file)
            .take((MAX_LEDGER_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
        if bytes.len() != length {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        Ok(Some(bytes))
    }

    pub(super) fn persist_owner_anchor(&self, bytes: &[u8]) -> Result<(), VoiceAuthorizationError> {
        if bytes.is_empty() || bytes.len() > MAX_LEDGER_BYTES {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        atomic_write_named(&self.directory, "owner-anchor.json", bytes)
    }
}

fn cleanup_temporary_files(directory: &File) -> Result<(), VoiceAuthorizationError> {
    // SAFETY: fcntl duplicates the dedicated directory descriptor for fdopendir ownership.
    let duplicate = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(VoiceAuthorizationError::StorageFailure);
    }
    // SAFETY: fdopendir takes ownership of duplicate on success.
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        unsafe { libc::close(duplicate) };
        return Err(VoiceAuthorizationError::StorageFailure);
    }
    let mut removed = false;
    loop {
        // SAFETY: stream is live and readdir returns entries owned by that stream.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        // SAFETY: dirent.d_name is NUL-terminated by readdir.
        let name_bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name_bytes.len() != 44
            || !name_bytes.starts_with(b".ledger-")
            || &name_bytes[40..] != b".tmp"
            || !name_bytes[8..40]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            continue;
        }
        let name = CString::new(name_bytes).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: fstatat checks a fixed directory-relative filename without following links.
        if unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            let code = std::io::Error::last_os_error().raw_os_error();
            if code == Some(libc::ENOENT) {
                continue;
            }
            unsafe { libc::closedir(stream) };
            return Err(VoiceAuthorizationError::StorageFailure);
        }
        // SAFETY: fstatat initialized stat after returning success.
        let stat = unsafe { stat.assume_init() };
        if stat.st_mode & libc::S_IFMT != libc::S_IFREG
            || stat.st_uid != unsafe { libc::geteuid() } as u32
            || stat.st_mode & 0o777 != PRIVATE_FILE_MODE as libc::mode_t
            || stat.st_nlink != 1
        {
            unsafe { libc::closedir(stream) };
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
        // SAFETY: this removes only a validated, reserved-prefix file in our private directory.
        if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            unsafe { libc::closedir(stream) };
            return Err(VoiceAuthorizationError::StorageFailure);
        }
        removed = true;
    }
    // SAFETY: closes stream and its duplicated descriptor.
    unsafe { libc::closedir(stream) };
    if removed && unsafe { libc::fsync(directory.as_raw_fd()) } != 0 {
        return Err(VoiceAuthorizationError::StorageFailure);
    }
    Ok(())
}

fn canonical_authorization_state(
    state: &SignedAuthorizationState,
) -> Result<Vec<u8>, VoiceAuthorizationError> {
    let value = serde_json::to_value(state).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
    serde_json::to_vec(&value).map_err(|_| VoiceAuthorizationError::StorageInvalid)
}

fn protected_state_matches(
    state: &SignedAuthorizationState,
    protected_state: Option<&[u8]>,
) -> Result<bool, VoiceAuthorizationError> {
    let Some(protected_state) = protected_state else {
        return Ok(false);
    };
    Ok(canonical_authorization_state(state)? == protected_state)
}

fn read_ledger(
    directory: &File,
    binding_sha256: &str,
) -> Result<Option<SignedLedgerEnvelope>, VoiceAuthorizationError> {
    let name = c_string("ledger.json")?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::ENOENT) => Ok(None),
            _ => Err(VoiceAuthorizationError::StorageFailure),
        };
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    validate_regular_file(&file, PRIVATE_FILE_MODE)?;
    let length = usize::try_from(
        file.metadata()
            .map_err(|_| VoiceAuthorizationError::StorageFailure)?
            .len(),
    )
    .map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
    if length == 0 || length > MAX_LEDGER_BYTES {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    let mut bytes = Vec::with_capacity(length);
    file.read_to_end(&mut bytes)
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    if bytes.len() != length {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    let envelope: SignedLedgerEnvelope =
        serde_json::from_slice(&bytes).map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
    let canonical = serde_json::to_value(&envelope)
        .and_then(|value| serde_json::to_vec(&value))
        .map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
    if canonical != bytes
        || envelope.schema != SIGNED_LEDGER_ENVELOPE_SCHEMA
        || envelope.authorization_state.signature_der.len() < 8
        || envelope.authorization_state.signature_der.len() > 256
        || envelope.authorization_state.signature_der.first() != Some(&0x30)
    {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    envelope.payload.validate(binding_sha256)?;
    envelope
        .authorization_state
        .payload
        .validate()
        .map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
    if envelope.authorization_state.payload != envelope.payload.authorization_state_payload()? {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    Ok(Some(envelope))
}

fn atomic_write(directory: &File, bytes: &[u8]) -> Result<(), VoiceAuthorizationError> {
    atomic_write_named(directory, "ledger.json", bytes)
}

fn atomic_write_named(
    directory: &File,
    target_name: &str,
    bytes: &[u8],
) -> Result<(), VoiceAuthorizationError> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    let temporary_name = format!(".ledger-{}.tmp", hex_lower(&random));
    let temporary_c = c_string(&temporary_name)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            temporary_c.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            PRIVATE_FILE_MODE,
        )
    };
    if fd < 0 {
        return Err(VoiceAuthorizationError::StorageFailure);
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let temporary_directory = directory
        .try_clone()
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    let mut temporary = TemporaryFile::new(temporary_directory, temporary_name, file)?;
    temporary
        .file
        .write_all(bytes)
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    temporary
        .file
        .sync_all()
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    let target = c_string(target_name)?;
    let temporary_c = c_string(&temporary.name)?;
    if unsafe {
        libc::renameat(
            directory.as_raw_fd(),
            temporary_c.as_ptr(),
            directory.as_raw_fd(),
            target.as_ptr(),
        )
    } != 0
    {
        return Err(VoiceAuthorizationError::StorageFailure);
    }
    temporary.renamed = true;
    if unsafe { libc::fsync(directory.as_raw_fd()) } != 0 {
        return Err(VoiceAuthorizationError::StorageFailure);
    }
    Ok(())
}

struct TemporaryFile {
    directory: File,
    name: String,
    file: File,
    identity: (u64, u64),
    renamed: bool,
}

impl TemporaryFile {
    fn new(directory: File, name: String, file: File) -> Result<Self, VoiceAuthorizationError> {
        validate_regular_file(&file, PRIVATE_FILE_MODE)?;
        let metadata = file
            .metadata()
            .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
        Ok(Self {
            directory,
            name,
            file,
            identity: (metadata.dev(), metadata.ino()),
            renamed: false,
        })
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if self.renamed {
            return;
        }
        let Ok(name) = CString::new(self.name.as_bytes()) else {
            return;
        };
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: fstatat writes a stat result for this directory-relative name.
        if unsafe {
            libc::fstatat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
        {
            // SAFETY: fstatat returned success and initialized metadata.
            let metadata = unsafe { metadata.assume_init() };
            if metadata.st_dev as u64 == self.identity.0 && metadata.st_ino == self.identity.1 {
                // SAFETY: unlinkat removes only the unique file created and inode-checked above.
                unsafe { libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0) };
            }
        }
    }
}

pub(super) fn open_relative_readonly(
    root: &Path,
    relative: &Path,
    expected_bytes: u64,
    private_user_file: bool,
) -> Result<File, VoiceAuthorizationError> {
    if expected_bytes == 0 || relative.is_absolute() {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    let mut directory = open_root_directory()?;
    let components = absolute_normal_components(&canonical_root)?;
    for component in components {
        directory = open_existing_directory(&directory, &component)?;
    }
    let root_metadata = directory
        .metadata()
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    if !root_metadata.is_dir()
        || (private_user_file
            && (root_metadata.uid() != unsafe { libc::geteuid() } as u32
                || root_metadata.mode() & 0o077 != 0))
    {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    let mut components = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => {
                let name = name
                    .to_str()
                    .ok_or(VoiceAuthorizationError::StorageInvalid)?;
                if name.is_empty() || name.contains('/') || name.contains('\0') {
                    return Err(VoiceAuthorizationError::StorageInvalid);
                }
                components.push(name.to_owned());
            }
            _ => return Err(VoiceAuthorizationError::StorageInvalid),
        }
    }
    let (leaf, parents) = components
        .split_last()
        .ok_or(VoiceAuthorizationError::StorageInvalid)?;
    for component in parents {
        directory = open_existing_directory(&directory, component)?;
        let metadata = directory
            .metadata()
            .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
        if private_user_file
            && (metadata.uid() != unsafe { libc::geteuid() } as u32
                || metadata.mode() & 0o777 != PRIVATE_DIR_MODE)
        {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
    }
    let leaf = c_string(leaf)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(VoiceAuthorizationError::StorageFailure);
    }
    // SAFETY: openat returned a fresh read-only descriptor now owned by File.
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    if !metadata.is_file()
        || metadata.len() != expected_bytes
        || metadata.nlink() != 1
        || (private_user_file
            && (metadata.uid() != unsafe { libc::geteuid() } as u32
                || metadata.mode() & 0o777 != READ_ONLY_MODEL_MODE))
    {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    Ok(file)
}

fn open_existing_directory(
    parent: &File,
    component: &str,
) -> Result<File, VoiceAuthorizationError> {
    let name = c_string(component)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(VoiceAuthorizationError::StorageFailure);
    }
    // SAFETY: openat returned a fresh directory descriptor now owned by File.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn open_voice_authorization_dir(app_data_dir: &Path) -> Result<File, VoiceAuthorizationError> {
    let mut current = open_root_directory()?;
    let components = absolute_normal_components(app_data_dir)?;
    for (index, component) in components.iter().enumerate() {
        current = open_or_create_directory(&current, component)?;
        if index + 1 == components.len() {
            validate_owned_directory(&current, None)?;
        }
    }
    let voice = open_or_create_directory(&current, "voice")?;
    validate_owned_directory(&voice, Some(PRIVATE_DIR_MODE))?;
    let authorization = open_or_create_directory(&voice, "authorization")?;
    validate_owned_directory(&authorization, Some(PRIVATE_DIR_MODE))?;
    Ok(authorization)
}

fn open_root_directory() -> Result<File, VoiceAuthorizationError> {
    let path = c"/";
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(VoiceAuthorizationError::StorageFailure);
    }
    // SAFETY: open returned an owned directory descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn absolute_normal_components(path: &Path) -> Result<Vec<String>, VoiceAuthorizationError> {
    if !path.is_absolute() {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    let mut result = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(value) => {
                let value = value
                    .to_str()
                    .ok_or(VoiceAuthorizationError::StorageInvalid)?;
                if value.is_empty() || value == "." || value == ".." || value.contains('/') {
                    return Err(VoiceAuthorizationError::StorageInvalid);
                }
                result.push(value.to_owned());
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(VoiceAuthorizationError::StorageInvalid)
            }
        }
    }
    if result.is_empty() {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    Ok(result)
}

fn open_or_create_directory(
    parent: &File,
    component: &str,
) -> Result<File, VoiceAuthorizationError> {
    let name = c_string(component)?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    let mut fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
        // SAFETY: parent is a checked directory and component is exactly one safe path segment.
        if unsafe {
            libc::mkdirat(
                parent.as_raw_fd(),
                name.as_ptr(),
                PRIVATE_DIR_MODE as libc::mode_t,
            )
        } != 0
            && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST)
        {
            return Err(VoiceAuthorizationError::StorageFailure);
        }
        fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd >= 0 {
            // SAFETY: fd was opened with O_DIRECTORY|O_NOFOLLOW and remains uniquely owned.
            if unsafe { libc::fchmod(fd, PRIVATE_DIR_MODE as libc::mode_t) } != 0 {
                unsafe { libc::close(fd) };
                return Err(VoiceAuthorizationError::StorageFailure);
            }
        }
    }
    if fd < 0 {
        return Err(VoiceAuthorizationError::StorageFailure);
    }
    // SAFETY: openat returned a valid directory descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn validate_owned_directory(
    file: &File,
    required_mode: Option<u32>,
) -> Result<(), VoiceAuthorizationError> {
    let metadata = file
        .metadata()
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } as u32 {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    if let Some(mode) = required_mode {
        if metadata.mode() & 0o777 != mode {
            return Err(VoiceAuthorizationError::StorageInvalid);
        }
    }
    Ok(())
}

fn read_private_named(
    directory: &File,
    filename: &str,
    maximum: usize,
) -> Result<Option<Vec<u8>>, VoiceAuthorizationError> {
    let name = c_string(filename)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::ENOENT) => Ok(None),
            _ => Err(VoiceAuthorizationError::StorageFailure),
        };
    }
    let file = unsafe { File::from_raw_fd(fd) };
    validate_regular_file(&file, PRIVATE_FILE_MODE)?;
    let length = usize::try_from(
        file.metadata()
            .map_err(|_| VoiceAuthorizationError::StorageFailure)?
            .len(),
    )
    .map_err(|_| VoiceAuthorizationError::StorageInvalid)?;
    if length == 0 || length > maximum {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    let mut bytes = Vec::with_capacity(length);
    (&file)
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    if bytes.len() != length {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    Ok(Some(bytes))
}

fn open_private_file(
    directory: &File,
    name: &str,
    flags: i32,
) -> Result<File, VoiceAuthorizationError> {
    let name = c_string(name)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            PRIVATE_FILE_MODE,
        )
    };
    if fd < 0 {
        return Err(VoiceAuthorizationError::StorageFailure);
    }
    // SAFETY: openat returned a fresh descriptor owned by File.
    let file = unsafe { File::from_raw_fd(fd) };
    validate_regular_file(&file, PRIVATE_FILE_MODE)?;
    Ok(file)
}

fn validate_regular_file(file: &File, expected_mode: u32) -> Result<(), VoiceAuthorizationError> {
    let metadata = file
        .metadata()
        .map_err(|_| VoiceAuthorizationError::StorageFailure)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() } as u32
        || metadata.mode() & 0o777 != expected_mode
        || metadata.nlink() != 1
    {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    Ok(())
}

fn c_string(value: &str) -> Result<CString, VoiceAuthorizationError> {
    if value.is_empty() || value.contains('/') || value.contains('\0') {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    CString::new(value).map_err(|_| VoiceAuthorizationError::StorageInvalid)
}

pub(super) fn decode_lower_hex(value: &str) -> Result<Vec<u8>, VoiceAuthorizationError> {
    if value.is_empty()
        || !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(VoiceAuthorizationError::StorageInvalid);
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().as_chunks::<2>().0 {
        let high = nibble(pair[0]).ok_or(VoiceAuthorizationError::StorageInvalid)?;
        let low = nibble(pair[1]).ok_or(VoiceAuthorizationError::StorageInvalid)?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

pub(super) fn encode_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_lower(bytes: &[u8]) -> String {
    encode_lower_hex(bytes)
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn valid_bundle_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.split('.').count() >= 2
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && part.as_bytes()[0].is_ascii_alphanumeric()
                && part.as_bytes()[part.len() - 1].is_ascii_alphanumeric()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::PathBuf;

    struct TestDirectory(PathBuf);

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn rejects_parent_traversal_and_symlinked_directory_components() {
        assert!(matches!(
            absolute_normal_components(Path::new("/tmp/../etc")),
            Err(VoiceAuthorizationError::StorageInvalid)
        ));
        let root = PathBuf::from("/tmp/saturday-voice-auth-test/link/child");
        assert!(absolute_normal_components(&root).is_ok());
        let directory = tempfile_dir_for_test();
        let outside = directory.0.join("outside");
        std::fs::create_dir(&outside).unwrap();
        let link = directory.0.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(matches!(
            open_or_create_directory(&File::open(&directory.0).unwrap(), "link"),
            Err(VoiceAuthorizationError::StorageFailure)
        ));
    }

    #[test]
    fn rejects_wrong_file_mode_and_hard_linked_ledger_files() {
        let directory = tempfile_dir_for_test();
        let file_path = directory.0.join("state");
        std::fs::write(&file_path, b"{}").unwrap();
        std::fs::set_permissions(&file_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let file = File::open(&file_path).unwrap();
        assert!(matches!(
            validate_regular_file(&file, PRIVATE_FILE_MODE),
            Err(VoiceAuthorizationError::StorageInvalid)
        ));
        std::fs::set_permissions(&file_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.0.join("link");
        std::fs::hard_link(&file_path, &link).unwrap();
        assert!(matches!(
            validate_regular_file(&file, PRIVATE_FILE_MODE),
            Err(VoiceAuthorizationError::StorageInvalid)
        ));
    }

    #[test]
    fn removes_only_reserved_crash_temporary_files_during_recovery() {
        let directory = tempfile_dir_for_test();
        let voice = directory.0.join("voice");
        let authorization_path = voice.join("authorization");
        std::fs::create_dir(&voice).unwrap();
        std::fs::create_dir(&authorization_path).unwrap();
        std::fs::set_permissions(&voice, std::fs::Permissions::from_mode(PRIVATE_DIR_MODE))
            .unwrap();
        std::fs::set_permissions(
            &authorization_path,
            std::fs::Permissions::from_mode(PRIVATE_DIR_MODE),
        )
        .unwrap();
        let authorization = File::open(authorization_path).unwrap();
        let orphan = ".ledger-00112233445566778899aabbccddeeff.tmp";
        let file = open_private_file(
            &authorization,
            orphan,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )
        .unwrap();
        drop(file);
        cleanup_temporary_files(&authorization).unwrap();
        assert!(matches!(
            open_private_file(&authorization, orphan, libc::O_RDONLY),
            Err(VoiceAuthorizationError::StorageFailure)
        ));
    }

    #[test]
    fn relative_open_rejects_symlink_hardlink_and_wrong_permissions() {
        let root = tempfile_dir_for_test();
        std::fs::create_dir(root.0.join("models")).unwrap();
        std::fs::set_permissions(
            root.0.join("models"),
            std::fs::Permissions::from_mode(PRIVATE_DIR_MODE),
        )
        .unwrap();
        let payload = root.0.join("models/payload");
        std::fs::write(&payload, b"data").unwrap();
        std::fs::set_permissions(
            &payload,
            std::fs::Permissions::from_mode(READ_ONLY_MODEL_MODE),
        )
        .unwrap();
        let result = open_relative_readonly(&root.0, Path::new("models/payload"), 4, true);
        let root_metadata = std::fs::metadata(&root.0).unwrap();
        let models_metadata = std::fs::metadata(root.0.join("models")).unwrap();
        let payload_metadata = std::fs::metadata(&payload).unwrap();
        assert!(
            result.is_ok(),
            "valid relative open failed: {result:?}; root uid={:?} mode={:?} len={:?} nlink={:?}; models uid={:?} mode={:?} len={:?} nlink={:?}; payload uid={:?} mode={:?} len={:?} nlink={:?}",
            root_metadata.uid(),
            root_metadata.mode() & 0o777,
            root_metadata.len(),
            root_metadata.nlink(),
            models_metadata.uid(),
            models_metadata.mode() & 0o777,
            models_metadata.len(),
            models_metadata.nlink(),
            payload_metadata.uid(),
            payload_metadata.mode() & 0o777,
            payload_metadata.len(),
            payload_metadata.nlink(),
        );
        let hard_link = root.0.join("models/hard-link");
        std::fs::hard_link(&payload, &hard_link).unwrap();
        assert!(open_relative_readonly(&root.0, Path::new("models/payload"), 4, true).is_err());
        std::fs::remove_file(hard_link).unwrap();
        std::fs::remove_file(&payload).unwrap();
        std::os::unix::fs::symlink(root.0.join("outside"), &payload).unwrap();
        assert!(open_relative_readonly(&root.0, Path::new("models/payload"), 4, true).is_err());
    }
    fn test_grant_record(generation: u64, grant_id: &str, nonce: &str) -> GrantRecord {
        let digest = "a".repeat(64);
        GrantRecord {
            payload: GrantPayload {
                schema: VOICE_GRANT_SCHEMA.to_owned(),
                request_role: "voice_asset_install".to_owned(),
                generation,
                grant_id_hex: grant_id.to_owned(),
                asset_id: "whisper-base".to_owned(),
                purpose: "asr_weights".to_owned(),
                worker_role: super::super::WorkerRole::Asr,
                expected_bytes: 147_951_465,
                expected_sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"
                    .to_owned(),
                binding_sha256: "b".repeat(64),
                source_generation_sha256: digest.clone(),
                source_tree_sha256: digest.clone(),
                resource_generation_sha256: digest.clone(),
                worker_sha256: digest.clone(),
                worker_bytes: 1024,
                worker_source_seal_sha256: digest.clone(),
                worker_resource_seal_sha256: digest.clone(),
                template_fingerprint_sha256: None,
                application_bundle_identifier: "dev.rohtolos.saturday".to_owned(),
                application_team_id_sha256: digest.clone(),
                application_requirement_sha256: digest.clone(),
                parent_executable_sha256: digest.clone(),
                parent_resource_closure_sha256: digest.clone(),
                worker_team_id_sha256: digest.clone(),
                worker_requirement_sha256: digest,
                issued_at_unix: 10,
                expires_at_unix: 20,
                use_expires_at_unix: 30,
                consent_nonce_sha256: nonce.to_owned(),
            },
            signature_der: vec![0x30; 8],
            state: GrantState::Cancelled,
            purpose_proof: None,
            promoted_version: None,
        }
    }

    fn test_ledger_with_history(history: Vec<GrantRecord>) -> DurableLedger {
        DurableLedger {
            schema: VOICE_LEDGER_SCHEMA.to_owned(),
            binding_sha256: "b".repeat(64),
            serial: 2,
            owner_public_key_x963: Some(encode_lower_hex(&[0x04; 65])),
            last_nonce_sha256: Some("c".repeat(64)),
            pending: None,
            history,
            active_assets: Vec::new(),
        }
    }

    #[test]
    fn rejects_replayed_grant_ids_nonces_and_wrong_worker_roles() {
        let first = test_grant_record(1, &"1".repeat(32), &"a".repeat(64));
        let duplicate_id = test_grant_record(2, &"1".repeat(32), &"b".repeat(64));
        assert!(test_ledger_with_history(vec![first.clone(), duplicate_id])
            .validate(&"b".repeat(64))
            .is_err());

        let duplicate_nonce = test_grant_record(2, &"2".repeat(32), &"a".repeat(64));
        assert!(
            test_ledger_with_history(vec![first.clone(), duplicate_nonce])
                .validate(&"b".repeat(64))
                .is_err()
        );

        let mut wrong_role = test_grant_record(1, &"3".repeat(32), &"c".repeat(64));
        wrong_role.payload.worker_role = super::super::WorkerRole::Llm;
        assert!(test_ledger_with_history(vec![wrong_role])
            .validate(&"b".repeat(64))
            .is_err());

        let mut malformed_signature = test_grant_record(1, &"4".repeat(32), &"d".repeat(64));
        malformed_signature.signature_der = vec![0x30; 257];
        assert!(test_ledger_with_history(vec![malformed_signature])
            .validate(&"b".repeat(64))
            .is_err());
    }

    #[test]
    fn atomic_snapshot_write_publishes_complete_private_file() {
        let directory = tempfile_dir_for_test();
        let directory_fd = File::open(&directory.0).unwrap();
        atomic_write_named(&directory_fd, "ledger.json", b"complete signed snapshot").unwrap();
        let path = directory.0.join("ledger.json");
        assert_eq!(std::fs::read(&path).unwrap(), b"complete signed snapshot");
        assert_eq!(
            std::fs::metadata(&path).unwrap().mode() & 0o777,
            PRIVATE_FILE_MODE
        );
        assert_eq!(
            std::fs::read_dir(&directory.0).unwrap().count(),
            1,
            "temporary atomic-write files must be removed after publication",
        );
    }

    fn tempfile_dir_for_test() -> TestDirectory {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!("saturday-voice-auth-{}", hex_lower(&random)));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(PRIVATE_DIR_MODE)).unwrap();
        TestDirectory(path)
    }

    #[test]
    fn ledger_is_strict_canonical_json_and_rejects_duplicate_members() {
        let duplicate = br#"{"schema":"saturday.voice.authorization-ledger.v1","schema":"saturday.voice.authorization-ledger.v1","binding_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","serial":0,"owner_public_key_x963":null,"last_nonce_sha256":null,"pending":null,"history":[],"active_assets":[]}"#;
        assert!(serde_json::from_slice::<DurableLedger>(duplicate).is_err());
        let ledger = DurableLedger::empty("a".repeat(64));
        let canonical = serde_json::to_vec(&ledger).unwrap();
        assert_eq!(
            serde_json::to_vec(&serde_json::from_slice::<DurableLedger>(&canonical).unwrap())
                .unwrap(),
            canonical
        );
    }

    #[test]
    fn lower_hex_codec_rejects_uppercase_and_malformed_values() {
        assert!(decode_lower_hex("AB").is_err());
        assert!(decode_lower_hex("0").is_err());
        assert_eq!(decode_lower_hex("00ff").unwrap(), [0, 255]);
        assert_eq!(encode_lower_hex(&[0, 255]), "00ff");
    }

    #[test]
    fn restored_old_signed_snapshot_does_not_match_protected_latest_state() {
        let signed_state = |serial, digest: &str| SignedAuthorizationState {
            payload: AuthorizationStatePayload {
                schema: parent_auth::AUTHORIZATION_STATE_SCHEMA.to_owned(),
                binding_sha256: "a".repeat(64),
                serial,
                ledger_sha256: digest.to_owned(),
                active_assets: Vec::new(),
            },
            signature_der: vec![0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        };
        let old_enabled_snapshot = signed_state(12, &"b".repeat(64));
        let latest_disabled_state = signed_state(13, &"c".repeat(64));
        let protected_latest = canonical_authorization_state(&latest_disabled_state).unwrap();

        assert!(!protected_state_matches(&old_enabled_snapshot, Some(&protected_latest)).unwrap());
        assert!(!protected_state_matches(&latest_disabled_state, None).unwrap());
        assert!(protected_state_matches(&latest_disabled_state, Some(&protected_latest)).unwrap());
    }
}
