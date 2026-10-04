use crate::owner_authorization::binding::{
    bootstrap_record_bytes, parse_frozen_binding, parse_public_key_anchor, BindingError,
    FrozenBinding, PublicKeyAnchor,
};
use crate::owner_authorization::canonical::canonical_json_bytes;
use crate::owner_authorization::cli::NativeMode;
use crate::owner_authorization::deletion::{
    delete_owner_key, LedgerStateGuard, LedgerStateReader, NativeBootstrapRecordStore,
    NativeLedgerStateReader,
};
use crate::owner_authorization::paths::{
    open_stage1_directories, Stage1Directories, SystemAccountHomeResolver,
};
use crate::owner_authorization::process::{
    NativeStage1ProcessRunner, Stage1ChildRequest, Stage1ProcessError, Stage1ProcessRunner,
};
use crate::owner_authorization::{
    AUTHORIZATION_TTL_SECONDS, ENVELOPE_SCHEMA, PAYLOAD_SCHEMA, SIGNING_DOMAIN,
};
use crate::platform::owner_authorization::{
    KeyProof, OwnerKeyBackend, OwnerKeyError, SecurityFrameworkOwnerKeyBackend,
};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum OwnerAuthorizationError {
    #[error("owner bundle is invalid")]
    Bundle,
    #[error("owner binding is invalid")]
    Binding,
    #[error("secure enclave is unavailable")]
    SecureEnclaveUnavailable,
    #[error("owner key was not found")]
    KeyNotFound,
    #[error("owner key does not match")]
    KeyMismatch,
    #[error("owner cancelled authentication")]
    UserCancelled,
    #[error("owner authentication failed")]
    Authentication,
    #[error("owner signature is invalid")]
    Signature,
    #[error("owner authorization expired")]
    Expired,
    #[error("canonical owner storage is invalid")]
    CanonicalStorage,
    #[error("owner authorization was already consumed")]
    AlreadyConsumed,
    #[error("owner authorization recovery failed")]
    Recovery,
    #[error("stage 1 Python launch failed")]
    PythonLaunch,
    #[error("stage 1 execution failed")]
    Stage1Failed,
    #[error("owner key deletion is refused")]
    DeleteRefused,
    #[error("owner key deletion completed but bootstrap cleanup is incomplete")]
    DeleteIncomplete,
}

impl OwnerAuthorizationError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Bundle => "bundle",
            Self::Binding => "binding",
            Self::SecureEnclaveUnavailable => "secure-enclave-unavailable",
            Self::KeyNotFound => "key-not-found",
            Self::KeyMismatch => "key-mismatch",
            Self::UserCancelled => "user-cancelled",
            Self::Authentication => "authentication",
            Self::Signature => "signature",
            Self::Expired => "expired",
            Self::CanonicalStorage => "canonical-storage",
            Self::AlreadyConsumed => "already-consumed",
            Self::Recovery => "recovery",
            Self::PythonLaunch => "python-launch",
            Self::Stage1Failed => "stage1-failed",
            Self::DeleteRefused => "delete-refused",
            Self::DeleteIncomplete => "delete-incomplete",
        }
    }
}

pub const BOOTSTRAP_CHALLENGE_DOMAIN: &[u8] = b"JARVIS-STAGE1-OWNER-KEY-BOOTSTRAP-V1\0";
pub const CONTINUITY_CHALLENGE_DOMAIN: &[u8] = b"JARVIS-STAGE1-OWNER-KEY-CONTINUITY-V1\0";
pub const CONTINUITY_PROMPT: &str = "Authenticate to prove J.A.R.V.I.S owner-key continuity";
pub const AUTHORIZATION_PROMPT: &str =
    "Authenticate to authorize the J.A.R.V.I.S Stage 1 bottle transfer";
const BOOTSTRAP_RECORD_NAME: &str = "owner-public-key.json";
const BOOTSTRAP_TEMP_NAME: &str = ".owner-public-key.json.tmp";
const MAX_BOOTSTRAP_RECORD_BYTES: usize = 4096;
const MAX_BUNDLED_BINDING_BYTES: usize = 256 * 1024;
const MAX_BUNDLED_SCRIPT_BYTES: u64 = 512 * 1024;
const MAX_SIGNATURE_BYTES: usize = 256;

pub struct IssueContext<'a> {
    pub binding: &'a FrozenBinding,
    pub directories: &'a Stage1Directories,
    pub now_unix: i64,
    pub generation: [u8; 16],
    pub nonce: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedAuthorization {
    pub envelope_bytes: Vec<u8>,
    pub nonce: [u8; 32],
    pub generation_hex: String,
    pub receipt_name: String,
}

pub fn build_and_sign_authorization(
    context: IssueContext<'_>,
    key_backend: &impl OwnerKeyBackend,
    anchor: &PublicKeyAnchor,
) -> Result<SignedAuthorization, OwnerAuthorizationError> {
    let (key_id, public_key_x963) = match anchor {
        PublicKeyAnchor::Unprovisioned => return Err(OwnerAuthorizationError::Binding),
        PublicKeyAnchor::Pinned {
            key_id,
            public_key_x963,
        } => (key_id, public_key_x963),
    };
    let expected_key_id: [u8; 32] = Sha256::digest(public_key_x963).into();
    if key_id != &expected_key_id {
        return Err(OwnerAuthorizationError::KeyMismatch);
    }
    let generation_hex = hex_lower(&context.generation);
    let receipt_name = crate::owner_authorization::paths::receipt_name(&generation_hex)
        .map_err(|_| OwnerAuthorizationError::Binding)?;
    let expires_at_unix = context
        .now_unix
        .checked_add(AUTHORIZATION_TTL_SECONDS)
        .ok_or(OwnerAuthorizationError::Binding)?;
    let nonce_sha256: [u8; 32] = Sha256::digest(context.nonce).into();
    let payload = serde_json::json!({
        "artifacts": binding_artifacts_json(context.binding),
        "binding_sha256": hex_lower(&context.binding.binding_sha256),
        "candidate": {
            "candidate_id": context.binding.candidate.candidate_id,
            "candidate_sha256": hex_lower(&context.binding.candidate.candidate_sha256),
            "review_aggregate_sha256": hex_lower(&context.binding.candidate.review_aggregate_sha256),
        },
        "expires_at_unix": expires_at_unix,
        "generation": generation_hex,
        "issued_at_unix": context.now_unix,
        "ledger": directory_identity_json(context.directories.ledger_identity),
        "limits": binding_limits_json(context.binding),
        "model_transfer": "blocked",
        "nonce_sha256": hex_lower(&nonce_sha256),
        "purpose": "stage1_bottle_transfer",
        "receipt": {
            "device": context.directories.receipt_parent_identity.device,
            "inode": context.directories.receipt_parent_identity.inode,
            "name": receipt_name,
            "uid": context.directories.receipt_parent_identity.uid,
        },
        "review": {
            "review_aggregate_sha256": hex_lower(&context.binding.review.review_aggregate_sha256),
            "review_manifest_sha256": hex_lower(&context.binding.review.review_manifest_sha256),
        },
        "schema": PAYLOAD_SCHEMA,
        "state": "granted",
        "workspace_parent": directory_identity_json(context.directories.workspace_parent_identity),
    });
    let payload_bytes =
        canonical_json_bytes(&payload).map_err(|_| OwnerAuthorizationError::Binding)?;
    let mut message = Vec::with_capacity(SIGNING_DOMAIN.len() + payload_bytes.len());
    message.extend_from_slice(SIGNING_DOMAIN);
    message.extend_from_slice(&payload_bytes);
    let proof = key_backend
        .sign(&message, AUTHORIZATION_PROMPT)
        .map_err(map_key_error)?;
    validate_key_proof(&proof)?;
    if proof.public_key_x963.as_slice() != public_key_x963 {
        return Err(OwnerAuthorizationError::KeyMismatch);
    }
    let envelope = serde_json::json!({
        "algorithm": crate::owner_authorization::binding::OWNER_SIGNATURE_ALGORITHM,
        "key_id": hex_lower(key_id),
        "payload_base64": STANDARD.encode(payload_bytes),
        "schema": ENVELOPE_SCHEMA,
        "signature_base64": STANDARD.encode(proof.signature_der),
    });
    let envelope_bytes =
        canonical_json_bytes(&envelope).map_err(|_| OwnerAuthorizationError::Binding)?;
    Ok(SignedAuthorization {
        envelope_bytes,
        nonce: context.nonce,
        generation_hex,
        receipt_name,
    })
}

pub fn issue_and_run_authorization(
    context: IssueContext<'_>,
    script: &Path,
    key_backend: &impl OwnerKeyBackend,
    anchor: &PublicKeyAnchor,
    process_runner: &impl Stage1ProcessRunner,
) -> Result<String, OwnerAuthorizationError> {
    let directories = context.directories;
    let signed = build_and_sign_authorization(context, key_backend, anchor)?;
    process_runner
        .run(Stage1ChildRequest {
            script,
            envelope: &signed.envelope_bytes,
            nonce: &signed.nonce,
            ledger_fd: directories.ledger.as_fd(),
            workspace_parent_fd: directories.workspace_parent.as_fd(),
            receipt_parent_fd: directories.receipt_parent.as_fd(),
        })
        .map_err(map_process_error)
}

fn map_process_error(error: Stage1ProcessError) -> OwnerAuthorizationError {
    match error {
        Stage1ProcessError::InvalidRequest | Stage1ProcessError::Launch => {
            OwnerAuthorizationError::PythonLaunch
        }
        Stage1ProcessError::Timeout
        | Stage1ProcessError::ChildFailed
        | Stage1ProcessError::InvalidOutput
        | Stage1ProcessError::SurvivingProcess
        | Stage1ProcessError::Cleanup => OwnerAuthorizationError::Stage1Failed,
    }
}

fn directory_identity_json(
    identity: crate::owner_authorization::paths::DirectoryIdentity,
) -> serde_json::Value {
    serde_json::json!({
        "device": identity.device,
        "inode": identity.inode,
        "uid": identity.uid,
    })
}

fn binding_artifacts_json(binding: &FrozenBinding) -> Vec<serde_json::Value> {
    binding
        .artifacts
        .iter()
        .map(|artifact| {
            let mut value = serde_json::Map::new();
            value.insert("formula".to_owned(), artifact.formula.clone().into());
            value.insert(
                "layer_sha256".to_owned(),
                hex_lower(&artifact.layer_sha256).into(),
            );
            value.insert("repository".to_owned(), artifact.repository.clone().into());
            value.insert("transfer_bytes".to_owned(), artifact.transfer_bytes.into());
            value.insert("version".to_owned(), artifact.version.clone().into());
            if let Some(installed_bytes) = artifact.installed_bytes {
                value.insert("installed_bytes".to_owned(), installed_bytes.into());
            }
            if let Some(maximum_extracted_bytes) = artifact.maximum_extracted_bytes {
                value.insert(
                    "maximum_extracted_bytes".to_owned(),
                    maximum_extracted_bytes.into(),
                );
            }
            if let Some(policy) = &artifact.post_install_execution {
                value.insert("post_install_execution".to_owned(), policy.clone().into());
            }
            serde_json::Value::Object(value)
        })
        .collect()
}

fn binding_limits_json(binding: &FrozenBinding) -> serde_json::Value {
    let limits = &binding.limits;
    let origin = &limits.origin_policy;
    serde_json::json!({
        "artifact_count": limits.artifact_count,
        "formula_execution_allowed": limits.formula_execution_allowed,
        "maximum_archive_entries": limits.maximum_archive_entries,
        "maximum_total_extracted_bytes": limits.maximum_total_extracted_bytes,
        "maximum_transfer_bytes": limits.maximum_transfer_bytes,
        "minimum_free_bytes_before_each_body_read": limits.minimum_free_bytes_before_each_body_read,
        "origin_policy": {
            "allowed_blob_redirect_origin": origin.allowed_blob_redirect_origin,
            "authorization_forwarding": origin.authorization_forwarding,
            "content_length_required": origin.content_length_required,
            "redirect_limit": origin.redirect_limit,
            "registry": origin.registry,
            "reject_redirect_from_blob_origin": origin.reject_redirect_from_blob_origin,
            "service": origin.service,
            "strip_authorization_on_cross_origin": origin.strip_authorization_on_cross_origin,
            "token_realm": origin.token_realm,
        },
        "post_install_execution_allowed": limits.post_install_execution_allowed,
        "verified_layer_total_bytes": limits.verified_layer_total_bytes,
    })
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

pub trait OwnerAuthorizationActions {
    fn provision(&self) -> Result<(), OwnerAuthorizationError>;
    fn prove(&self) -> Result<(), OwnerAuthorizationError>;
    fn issue_and_run(&self) -> Result<String, OwnerAuthorizationError>;
    fn delete(&self) -> Result<(), OwnerAuthorizationError>;
}

pub fn dispatch_owner_mode(
    mode: &NativeMode,
    actions: &impl OwnerAuthorizationActions,
) -> Result<Option<String>, OwnerAuthorizationError> {
    match mode {
        NativeMode::Stage1OwnerKeyProvision => {
            actions.provision()?;
            Ok(Some("JARVIS_STAGE1_OWNER_KEY_PROVISIONED".to_owned()))
        }
        NativeMode::Stage1OwnerKeyProve => {
            actions.prove()?;
            Ok(Some("JARVIS_STAGE1_OWNER_KEY_CONTINUITY_OK".to_owned()))
        }
        NativeMode::Stage1OwnerAuthorizationIssue => {
            let digest = actions.issue_and_run()?;
            Ok(Some(format!(
                "JARVIS_STAGE1_OWNER_AUTHORIZATION_OK:{digest}"
            )))
        }
        NativeMode::Stage1OwnerKeyDelete => {
            actions.delete()?;
            Ok(Some("JARVIS_STAGE1_OWNER_KEY_DELETED".to_owned()))
        }
        NativeMode::Ordinary
        | NativeMode::VerifySignedHelper
        | NativeMode::MeasureMainThread(_) => Ok(None),
    }
}

pub fn provision_owner_key(
    key_backend: &impl OwnerKeyBackend,
    directories: &Stage1Directories,
    nonce: [u8; 32],
) -> Result<(), OwnerAuthorizationError> {
    let challenge = challenge_bytes(BOOTSTRAP_CHALLENGE_DOMAIN, nonce);
    let proof = key_backend.provision(&challenge).map_err(map_key_error)?;
    validate_key_proof(&proof)?;
    let record = bootstrap_record_bytes(&proof.public_key_x963).map_err(map_binding_error)?;
    write_bootstrap_record_exclusive_or_same(&directories.bootstrap, &record)
}

pub fn prove_owner_key(
    key_backend: &impl OwnerKeyBackend,
    anchor: &PublicKeyAnchor,
    nonce: [u8; 32],
) -> Result<(), OwnerAuthorizationError> {
    let (key_id, pinned_public_key) = match anchor {
        PublicKeyAnchor::Unprovisioned => return Err(OwnerAuthorizationError::Binding),
        PublicKeyAnchor::Pinned {
            key_id,
            public_key_x963,
        } => (key_id, public_key_x963),
    };
    let expected_key_id: [u8; 32] = Sha256::digest(pinned_public_key).into();
    if key_id != &expected_key_id {
        return Err(OwnerAuthorizationError::KeyMismatch);
    }

    let copied_public_key = key_backend.copy_public_key().map_err(map_key_error)?;
    validate_x963_public_key(&copied_public_key)?;
    if copied_public_key.as_slice() != pinned_public_key {
        return Err(OwnerAuthorizationError::KeyMismatch);
    }

    let challenge = challenge_bytes(CONTINUITY_CHALLENGE_DOMAIN, nonce);
    let proof = key_backend
        .sign(&challenge, CONTINUITY_PROMPT)
        .map_err(map_key_error)?;
    validate_key_proof(&proof)?;
    if proof.public_key_x963.as_slice() != pinned_public_key {
        return Err(OwnerAuthorizationError::KeyMismatch);
    }
    Ok(())
}

fn challenge_bytes(domain: &[u8], nonce: [u8; 32]) -> Vec<u8> {
    let mut challenge = Vec::with_capacity(domain.len() + nonce.len());
    challenge.extend_from_slice(domain);
    challenge.extend_from_slice(&nonce);
    challenge
}

fn validate_key_proof(proof: &KeyProof) -> Result<(), OwnerAuthorizationError> {
    validate_x963_public_key(&proof.public_key_x963)?;
    if proof.signature_der.len() < 2
        || proof.signature_der.len() > MAX_SIGNATURE_BYTES
        || proof.signature_der[0] != 0x30
    {
        return Err(OwnerAuthorizationError::Signature);
    }
    Ok(())
}

fn validate_x963_public_key(public_key: &[u8]) -> Result<(), OwnerAuthorizationError> {
    if public_key.len() != 65 || public_key[0] != 0x04 {
        return Err(OwnerAuthorizationError::KeyMismatch);
    }
    Ok(())
}

fn map_key_error(error: OwnerKeyError) -> OwnerAuthorizationError {
    match error {
        OwnerKeyError::SecureEnclaveUnavailable => {
            OwnerAuthorizationError::SecureEnclaveUnavailable
        }
        OwnerKeyError::KeyNotFound => OwnerAuthorizationError::KeyNotFound,
        OwnerKeyError::DuplicateKey | OwnerKeyError::InvalidPublicKey => {
            OwnerAuthorizationError::KeyMismatch
        }
        OwnerKeyError::UserCancelled => OwnerAuthorizationError::UserCancelled,
        OwnerKeyError::AuthenticationFailed => OwnerAuthorizationError::Authentication,
        OwnerKeyError::InvalidSignature => OwnerAuthorizationError::Signature,
        OwnerKeyError::System => OwnerAuthorizationError::Stage1Failed,
    }
}

fn map_binding_error(error: BindingError) -> OwnerAuthorizationError {
    match error {
        BindingError::InvalidRecord => OwnerAuthorizationError::Binding,
        BindingError::InvalidPublicKey => OwnerAuthorizationError::KeyMismatch,
    }
}

pub fn write_bootstrap_record_exclusive_or_same(
    directory: &OwnedFd,
    record: &[u8],
) -> Result<(), OwnerAuthorizationError> {
    if record.is_empty() || record.len() > MAX_BOOTSTRAP_RECORD_BYTES {
        return Err(OwnerAuthorizationError::CanonicalStorage);
    }
    let temporary =
        CString::new(BOOTSTRAP_TEMP_NAME).map_err(|_| OwnerAuthorizationError::CanonicalStorage)?;
    let final_name = CString::new(BOOTSTRAP_RECORD_NAME)
        .map_err(|_| OwnerAuthorizationError::CanonicalStorage)?;
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            temporary.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(OwnerAuthorizationError::CanonicalStorage);
    }
    let mut temporary_file = unsafe { File::from_raw_fd(descriptor) };
    let write_result = (|| {
        if unsafe { libc::fchmod(temporary_file.as_raw_fd(), 0o600) } != 0 {
            return Err(OwnerAuthorizationError::CanonicalStorage);
        }
        temporary_file
            .write_all(record)
            .map_err(|_| OwnerAuthorizationError::CanonicalStorage)?;
        temporary_file
            .sync_all()
            .map_err(|_| OwnerAuthorizationError::CanonicalStorage)
    })();
    drop(temporary_file);
    if let Err(error) = write_result {
        unlink_at(directory, &temporary);
        return Err(error);
    }

    match rename_exclusive(directory, &temporary, &final_name) {
        Ok(()) => {
            sync_directory(directory)?;
            Ok(())
        }
        Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
            unlink_at(directory, &temporary);
            read_existing_bootstrap(directory, &final_name, record)
        }
        Err(_) => {
            unlink_at(directory, &temporary);
            Err(OwnerAuthorizationError::CanonicalStorage)
        }
    }
}

#[cfg(target_os = "macos")]
fn rename_exclusive(
    directory: &OwnedFd,
    source: &CString,
    destination: &CString,
) -> Result<(), std::io::Error> {
    let status = unsafe {
        libc::renameatx_np(
            directory.as_raw_fd(),
            source.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn rename_exclusive(
    directory: &OwnedFd,
    source: &CString,
    destination: &CString,
) -> Result<(), std::io::Error> {
    let status = unsafe {
        libc::linkat(
            directory.as_raw_fd(),
            source.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
            0,
        )
    };
    if status != 0 {
        return Err(std::io::Error::last_os_error());
    }
    unlink_at(directory, source);
    Ok(())
}

fn read_existing_bootstrap(
    directory: &OwnedFd,
    name: &CString,
    expected: &[u8],
) -> Result<(), OwnerAuthorizationError> {
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(OwnerAuthorizationError::CanonicalStorage);
    }
    let mut file = unsafe { File::from_raw_fd(descriptor) };
    let metadata = file
        .metadata()
        .map_err(|_| OwnerAuthorizationError::CanonicalStorage)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o600
        || metadata.len() == 0
        || metadata.len() > MAX_BOOTSTRAP_RECORD_BYTES as u64
    {
        return Err(OwnerAuthorizationError::CanonicalStorage);
    }
    let bytes = read_bounded(&mut file, MAX_BOOTSTRAP_RECORD_BYTES)?;
    if bytes == expected {
        Ok(())
    } else {
        Err(OwnerAuthorizationError::KeyMismatch)
    }
}

fn unlink_at(directory: &OwnedFd, name: &CString) {
    unsafe {
        libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0);
    }
}

fn sync_directory(directory: &OwnedFd) -> Result<(), OwnerAuthorizationError> {
    if unsafe { libc::fsync(directory.as_raw_fd()) } == 0 {
        Ok(())
    } else {
        Err(OwnerAuthorizationError::CanonicalStorage)
    }
}

fn read_bounded(
    reader: &mut impl Read,
    maximum_bytes: usize,
) -> Result<Vec<u8>, OwnerAuthorizationError> {
    let limit = u64::try_from(maximum_bytes)
        .map_err(|_| OwnerAuthorizationError::CanonicalStorage)?
        .saturating_add(1);
    let mut bytes = Vec::new();
    reader
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| OwnerAuthorizationError::CanonicalStorage)?;
    if bytes.len() > maximum_bytes {
        return Err(OwnerAuthorizationError::CanonicalStorage);
    }
    Ok(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BundledStage1Paths {
    root: PathBuf,
    script: PathBuf,
    binding: PathBuf,
    public_key: PathBuf,
}

struct BundledStage1Resources {
    paths: BundledStage1Paths,
    binding: FrozenBinding,
    public_key: PublicKeyAnchor,
}

fn bundled_stage1_paths(executable: &Path) -> Result<BundledStage1Paths, OwnerAuthorizationError> {
    if !executable.is_absolute() {
        return Err(OwnerAuthorizationError::Bundle);
    }
    let macos = executable.parent().ok_or(OwnerAuthorizationError::Bundle)?;
    if macos.file_name().and_then(|value| value.to_str()) != Some("MacOS") {
        return Err(OwnerAuthorizationError::Bundle);
    }
    let contents = macos.parent().ok_or(OwnerAuthorizationError::Bundle)?;
    if contents.file_name().and_then(|value| value.to_str()) != Some("Contents") {
        return Err(OwnerAuthorizationError::Bundle);
    }
    let root = contents.join("Resources").join("jarvis").join("stage1");
    Ok(BundledStage1Paths {
        script: root.join("prebuilt_runtime_stage1.py"),
        binding: root.join("stage1-owner-binding.json"),
        public_key: root.join("stage1-owner-public-key.json"),
        root,
    })
}

fn load_bundled_file(
    path: &Path,
    maximum_bytes: usize,
) -> Result<Vec<u8>, OwnerAuthorizationError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| OwnerAuthorizationError::Bundle)?;
    let metadata = file
        .metadata()
        .map_err(|_| OwnerAuthorizationError::Bundle)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.len() == 0
        || metadata.len() > maximum_bytes as u64
    {
        return Err(OwnerAuthorizationError::Bundle);
    }
    read_bounded(&mut file, maximum_bytes).map_err(|_| OwnerAuthorizationError::Bundle)
}

fn verify_bundled_script(path: &Path) -> Result<(), OwnerAuthorizationError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| OwnerAuthorizationError::Bundle)?;
    let metadata = file
        .metadata()
        .map_err(|_| OwnerAuthorizationError::Bundle)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.len() == 0
        || metadata.len() > MAX_BUNDLED_SCRIPT_BYTES
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(OwnerAuthorizationError::Bundle);
    }
    Ok(())
}

fn load_bundled_stage1_resources() -> Result<BundledStage1Resources, OwnerAuthorizationError> {
    let executable = std::env::current_exe().map_err(|_| OwnerAuthorizationError::Bundle)?;
    let paths = bundled_stage1_paths(&executable)?;
    verify_bundled_script(&paths.script)?;
    let binding = parse_frozen_binding(&load_bundled_file(
        &paths.binding,
        MAX_BUNDLED_BINDING_BYTES,
    )?)
    .map_err(map_binding_error)?;
    let public_key = parse_public_key_anchor(&load_bundled_file(
        &paths.public_key,
        MAX_BOOTSTRAP_RECORD_BYTES,
    )?)
    .map_err(map_binding_error)?;
    Ok(BundledStage1Resources {
        paths,
        binding,
        public_key,
    })
}

fn load_bundled_public_key_anchor() -> Result<PublicKeyAnchor, OwnerAuthorizationError> {
    let executable = std::env::current_exe().map_err(|_| OwnerAuthorizationError::Bundle)?;
    let paths = bundled_stage1_paths(&executable)?;
    parse_public_key_anchor(&load_bundled_file(
        &paths.public_key,
        MAX_BOOTSTRAP_RECORD_BYTES,
    )?)
    .map_err(map_binding_error)
}

fn random_bytes<const LENGTH: usize>() -> Result<[u8; LENGTH], OwnerAuthorizationError> {
    let mut bytes = [0_u8; LENGTH];
    getrandom::fill(&mut bytes).map_err(|_| OwnerAuthorizationError::Recovery)?;
    Ok(bytes)
}

fn random_nonce() -> Result<[u8; 32], OwnerAuthorizationError> {
    random_bytes()
}

fn current_unix_time() -> Result<i64, OwnerAuthorizationError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OwnerAuthorizationError::Recovery)?
        .as_secs();
    i64::try_from(seconds).map_err(|_| OwnerAuthorizationError::Recovery)
}

pub struct NativeOwnerAuthorizationActions {
    key_backend: SecurityFrameworkOwnerKeyBackend,
    process_runner: NativeStage1ProcessRunner,
    ledger_reader: NativeLedgerStateReader,
    bootstrap_store: NativeBootstrapRecordStore,
}

impl NativeOwnerAuthorizationActions {
    pub fn new() -> Self {
        Self {
            key_backend: SecurityFrameworkOwnerKeyBackend::new(),
            process_runner: NativeStage1ProcessRunner,
            ledger_reader: NativeLedgerStateReader,
            bootstrap_store: NativeBootstrapRecordStore,
        }
    }
}

impl Default for NativeOwnerAuthorizationActions {
    fn default() -> Self {
        Self::new()
    }
}

impl OwnerAuthorizationActions for NativeOwnerAuthorizationActions {
    fn provision(&self) -> Result<(), OwnerAuthorizationError> {
        let directories = open_stage1_directories(&SystemAccountHomeResolver)
            .map_err(|_| OwnerAuthorizationError::CanonicalStorage)?;
        provision_owner_key(&self.key_backend, &directories, random_nonce()?)
    }

    fn prove(&self) -> Result<(), OwnerAuthorizationError> {
        let anchor = load_bundled_public_key_anchor()?;
        prove_owner_key(&self.key_backend, &anchor, random_nonce()?)
    }

    fn issue_and_run(&self) -> Result<String, OwnerAuthorizationError> {
        let resources = load_bundled_stage1_resources()?;
        let directories = open_stage1_directories(&SystemAccountHomeResolver)
            .map_err(|_| OwnerAuthorizationError::CanonicalStorage)?;
        {
            let guard = self
                .ledger_reader
                .lock_recover_and_summarize(&directories)?;
            let summary = guard.summary();
            if !summary.valid
                || !summary.recovery_complete
                || summary.active_or_prepared_generations != 0
            {
                return Err(OwnerAuthorizationError::Recovery);
            }
        }
        issue_and_run_authorization(
            IssueContext {
                binding: &resources.binding,
                directories: &directories,
                now_unix: current_unix_time()?,
                generation: random_bytes()?,
                nonce: random_nonce()?,
            },
            &resources.paths.script,
            &self.key_backend,
            &resources.public_key,
            &self.process_runner,
        )
    }

    fn delete(&self) -> Result<(), OwnerAuthorizationError> {
        let anchor = load_bundled_public_key_anchor()?;
        let directories = open_stage1_directories(&SystemAccountHomeResolver)
            .map_err(|_| OwnerAuthorizationError::CanonicalStorage)?;
        delete_owner_key(
            &directories,
            &self.key_backend,
            &anchor,
            &self.ledger_reader,
            &self.bootstrap_store,
            random_nonce()?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner_authorization::binding::{bootstrap_record_bytes, PublicKeyAnchor};
    use crate::owner_authorization::paths::{
        open_stage1_directories, AccountHomeResolver, PathError,
    };
    use crate::platform::owner_authorization::{KeyProof, OwnerKeyBackend, OwnerKeyError};
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeActions {
        provision: AtomicUsize,
        prove: AtomicUsize,
        issue: AtomicUsize,
        delete: AtomicUsize,
    }

    impl OwnerAuthorizationActions for FakeActions {
        fn provision(&self) -> Result<(), OwnerAuthorizationError> {
            self.provision.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn prove(&self) -> Result<(), OwnerAuthorizationError> {
            self.prove.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn issue_and_run(&self) -> Result<String, OwnerAuthorizationError> {
            self.issue.fetch_add(1, Ordering::SeqCst);
            Ok("11".repeat(32))
        }

        fn delete(&self) -> Result<(), OwnerAuthorizationError> {
            self.delete.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum KeyCall {
        Provision(Vec<u8>),
        CopyPublic,
        Sign { message: Vec<u8>, prompt: String },
    }

    struct FakeKeyBackend {
        calls: Arc<Mutex<Vec<KeyCall>>>,
        provision_result: Result<KeyProof, OwnerKeyError>,
        copy_result: Result<Vec<u8>, OwnerKeyError>,
        sign_result: Result<KeyProof, OwnerKeyError>,
    }

    impl OwnerKeyBackend for FakeKeyBackend {
        fn provision(&self, challenge: &[u8]) -> Result<KeyProof, OwnerKeyError> {
            self.calls
                .lock()
                .unwrap()
                .push(KeyCall::Provision(challenge.to_vec()));
            self.provision_result.clone()
        }

        fn copy_public_key(&self) -> Result<Vec<u8>, OwnerKeyError> {
            self.calls.lock().unwrap().push(KeyCall::CopyPublic);
            self.copy_result.clone()
        }

        fn sign(&self, message: &[u8], prompt: &str) -> Result<KeyProof, OwnerKeyError> {
            self.calls.lock().unwrap().push(KeyCall::Sign {
                message: message.to_vec(),
                prompt: prompt.to_owned(),
            });
            self.sign_result.clone()
        }

        fn delete_after_proof(
            &self,
            _challenge: &[u8],
            _prompt: &str,
        ) -> Result<(), OwnerKeyError> {
            Err(OwnerKeyError::System)
        }
    }

    struct FakeResolver(PathBuf);

    impl AccountHomeResolver for FakeResolver {
        fn effective_user_home(&self) -> Result<PathBuf, PathError> {
            Ok(self.0.clone())
        }
    }

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);
            let path = std::env::temp_dir().join(format!(
                "jarvis-owner-issuer-{label}-{}-{}",
                std::process::id(),
                NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn key(byte: u8) -> Vec<u8> {
        let mut key = vec![byte; 65];
        key[0] = 0x04;
        key
    }

    fn proof(byte: u8) -> KeyProof {
        KeyProof {
            public_key_x963: key(byte),
            signature_der: vec![0x30, 0x00],
        }
    }

    fn fake_backend(
        provision_result: Result<KeyProof, OwnerKeyError>,
        copy_result: Result<Vec<u8>, OwnerKeyError>,
        sign_result: Result<KeyProof, OwnerKeyError>,
    ) -> (FakeKeyBackend, Arc<Mutex<Vec<KeyCall>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            FakeKeyBackend {
                calls: calls.clone(),
                provision_result,
                copy_result,
                sign_result,
            },
            calls,
        )
    }

    #[test]
    fn dispatches_each_owner_mode_to_one_exact_action_and_success_token() {
        let actions = FakeActions::default();
        assert_eq!(
            dispatch_owner_mode(&NativeMode::Stage1OwnerKeyProvision, &actions).unwrap(),
            Some("JARVIS_STAGE1_OWNER_KEY_PROVISIONED".to_owned())
        );
        assert_eq!(
            dispatch_owner_mode(&NativeMode::Stage1OwnerKeyProve, &actions).unwrap(),
            Some("JARVIS_STAGE1_OWNER_KEY_CONTINUITY_OK".to_owned())
        );
        assert_eq!(
            dispatch_owner_mode(&NativeMode::Stage1OwnerAuthorizationIssue, &actions).unwrap(),
            Some(format!(
                "JARVIS_STAGE1_OWNER_AUTHORIZATION_OK:{}",
                "11".repeat(32)
            ))
        );
        assert_eq!(
            dispatch_owner_mode(&NativeMode::Stage1OwnerKeyDelete, &actions).unwrap(),
            Some("JARVIS_STAGE1_OWNER_KEY_DELETED".to_owned())
        );
        assert_eq!(actions.provision.load(Ordering::SeqCst), 1);
        assert_eq!(actions.prove.load(Ordering::SeqCst), 1);
        assert_eq!(actions.issue.load(Ordering::SeqCst), 1);
        assert_eq!(actions.delete.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn ordinary_helper_and_probe_modes_perform_no_owner_action() {
        let actions = FakeActions::default();
        for mode in [
            NativeMode::Ordinary,
            NativeMode::VerifySignedHelper,
            NativeMode::MeasureMainThread(PathBuf::from("/private/tmp/native-main-thread.json")),
        ] {
            assert_eq!(dispatch_owner_mode(&mode, &actions).unwrap(), None);
        }
        assert_eq!(actions.provision.load(Ordering::SeqCst), 0);
        assert_eq!(actions.prove.load(Ordering::SeqCst), 0);
        assert_eq!(actions.issue.load(Ordering::SeqCst), 0);
        assert_eq!(actions.delete.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn provision_writes_exact_private_bootstrap_idempotently_and_rejects_mismatch() {
        let root = TestRoot::new("provision");
        let directories =
            open_stage1_directories(&FakeResolver(root.path().to_path_buf())).unwrap();
        let nonce = [0xa5; 32];
        let (backend, calls) = fake_backend(Ok(proof(0x11)), Ok(key(0x11)), Ok(proof(0x11)));

        provision_owner_key(&backend, &directories, nonce).unwrap();
        let mut expected_challenge = BOOTSTRAP_CHALLENGE_DOMAIN.to_vec();
        expected_challenge.extend_from_slice(&nonce);
        assert_eq!(
            *calls.lock().unwrap(),
            [KeyCall::Provision(expected_challenge)]
        );

        let bootstrap_path = root
            .path()
            .join("Library/Application Support/dev.rohtolos.jarvis/Stage1OwnerAuthorization/bootstrap/owner-public-key.json");
        let expected_record = bootstrap_record_bytes(&key(0x11)).unwrap();
        assert_eq!(fs::read(&bootstrap_path).unwrap(), expected_record);
        let metadata = fs::symlink_metadata(&bootstrap_path).unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(metadata.nlink(), 1);

        provision_owner_key(&backend, &directories, nonce).unwrap();
        let (mismatch, _) = fake_backend(Ok(proof(0x22)), Ok(key(0x22)), Ok(proof(0x22)));
        assert_eq!(
            provision_owner_key(&mismatch, &directories, nonce),
            Err(OwnerAuthorizationError::KeyMismatch)
        );
        assert_eq!(fs::read(&bootstrap_path).unwrap(), expected_record);
    }

    #[test]
    fn provision_failure_writes_no_bootstrap_record() {
        let root = TestRoot::new("provision-failure");
        let directories =
            open_stage1_directories(&FakeResolver(root.path().to_path_buf())).unwrap();
        let (backend, _) = fake_backend(
            Err(OwnerKeyError::UserCancelled),
            Ok(key(0x11)),
            Ok(proof(0x11)),
        );
        assert_eq!(
            provision_owner_key(&backend, &directories, [0x33; 32]),
            Err(OwnerAuthorizationError::UserCancelled)
        );
        let bootstrap = root.path().join(
            "Library/Application Support/dev.rohtolos.jarvis/Stage1OwnerAuthorization/bootstrap",
        );
        assert_eq!(fs::read_dir(bootstrap).unwrap().count(), 0);
    }

    #[test]
    fn continuity_proof_requires_and_matches_the_pinned_key_before_and_after_signing() {
        let record = bootstrap_record_bytes(&key(0x11)).unwrap();
        let anchor = crate::owner_authorization::binding::parse_bootstrap_record(&record).unwrap();
        let nonce = [0x5a; 32];
        let (backend, calls) = fake_backend(Ok(proof(0x11)), Ok(key(0x11)), Ok(proof(0x11)));
        prove_owner_key(&backend, &anchor, nonce).unwrap();
        let mut expected_challenge = CONTINUITY_CHALLENGE_DOMAIN.to_vec();
        expected_challenge.extend_from_slice(&nonce);
        assert_eq!(
            *calls.lock().unwrap(),
            [
                KeyCall::CopyPublic,
                KeyCall::Sign {
                    message: expected_challenge,
                    prompt: CONTINUITY_PROMPT.to_owned(),
                },
            ]
        );

        let (unprovisioned, unprovisioned_calls) =
            fake_backend(Ok(proof(0x11)), Ok(key(0x11)), Ok(proof(0x11)));
        assert_eq!(
            prove_owner_key(&unprovisioned, &PublicKeyAnchor::Unprovisioned, nonce),
            Err(OwnerAuthorizationError::Binding)
        );
        assert!(unprovisioned_calls.lock().unwrap().is_empty());

        let (copy_mismatch, copy_calls) =
            fake_backend(Ok(proof(0x11)), Ok(key(0x22)), Ok(proof(0x11)));
        assert_eq!(
            prove_owner_key(&copy_mismatch, &anchor, nonce),
            Err(OwnerAuthorizationError::KeyMismatch)
        );
        assert_eq!(*copy_calls.lock().unwrap(), [KeyCall::CopyPublic]);

        let (sign_mismatch, _) = fake_backend(Ok(proof(0x11)), Ok(key(0x11)), Ok(proof(0x22)));
        assert_eq!(
            prove_owner_key(&sign_mismatch, &anchor, nonce),
            Err(OwnerAuthorizationError::KeyMismatch)
        );
    }

    #[test]
    fn builds_exact_owner_authorization_payload_and_signs_domain_separated_bytes() {
        const BINDING: &[u8] =
            include_bytes!("../../../scripts/tests/fixtures/stage1-owner-binding-frozen.json");
        let binding = crate::owner_authorization::binding::parse_frozen_binding(BINDING).unwrap();
        let root = TestRoot::new("issue-payload");
        let directories =
            open_stage1_directories(&FakeResolver(root.path().to_path_buf())).unwrap();
        let generation = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let nonce = [0x5a; 32];
        let (backend, calls) = fake_backend(Ok(proof(0x11)), Ok(key(0x11)), Ok(proof(0x11)));
        let record = bootstrap_record_bytes(&key(0x11)).unwrap();
        let anchor = crate::owner_authorization::binding::parse_bootstrap_record(&record).unwrap();

        let signed = build_and_sign_authorization(
            IssueContext {
                binding: &binding,
                directories: &directories,
                now_unix: 1_795_000_000,
                generation,
                nonce,
            },
            &backend,
            &anchor,
        )
        .unwrap();

        let envelope: serde_json::Value = serde_json::from_slice(&signed.envelope_bytes).unwrap();
        let payload_bytes = base64::engine::general_purpose::STANDARD
            .decode(envelope["payload_base64"].as_str().unwrap())
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();
        assert_eq!(
            payload
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            [
                "artifacts",
                "binding_sha256",
                "candidate",
                "expires_at_unix",
                "generation",
                "issued_at_unix",
                "ledger",
                "limits",
                "model_transfer",
                "nonce_sha256",
                "purpose",
                "receipt",
                "review",
                "schema",
                "state",
                "workspace_parent",
            ]
        );
        assert_eq!(payload["generation"], "00112233445566778899aabbccddeeff");
        assert_eq!(payload["issued_at_unix"], 1_795_000_000_i64);
        assert_eq!(payload["expires_at_unix"], 1_795_000_300_i64);
        assert_eq!(payload["model_transfer"], "blocked");
        assert_eq!(payload["artifacts"].as_array().unwrap().len(), 5);
        assert_eq!(payload["artifacts"][0]["formula"], "llama.cpp");
        assert_eq!(payload["artifacts"][4]["formula"], "ca-certificates");
        assert_eq!(payload["limits"]["artifact_count"], 5);
        assert_eq!(
            payload["receipt"]["name"],
            "stage1-00112233445566778899aabbccddeeff.json"
        );
        assert_eq!(
            payload["ledger"],
            serde_json::json!({
                "device": directories.ledger_identity.device,
                "inode": directories.ledger_identity.inode,
                "uid": directories.ledger_identity.uid,
            })
        );
        assert_eq!(
            payload["workspace_parent"],
            serde_json::json!({
                "device": directories.workspace_parent_identity.device,
                "inode": directories.workspace_parent_identity.inode,
                "uid": directories.workspace_parent_identity.uid,
            })
        );
        assert_eq!(
            payload["receipt"]["device"],
            directories.receipt_parent_identity.device
        );
        assert_eq!(signed.nonce, nonce);
        assert_eq!(signed.generation_hex, "00112233445566778899aabbccddeeff");
        assert_eq!(
            signed.receipt_name,
            "stage1-00112233445566778899aabbccddeeff.json"
        );
        let mut expected_message = SIGNING_DOMAIN.to_vec();
        expected_message.extend_from_slice(&payload_bytes);
        assert_eq!(
            *calls.lock().unwrap(),
            [KeyCall::Sign {
                message: expected_message,
                prompt: AUTHORIZATION_PROMPT.to_owned(),
            }]
        );
    }

    #[test]
    fn shared_full_payload_and_envelope_fixtures_are_canonical_and_linked() {
        const PAYLOAD: &[u8] =
            include_bytes!("../../../scripts/tests/fixtures/stage1-owner-payload.json");
        const ENVELOPE: &[u8] =
            include_bytes!("../../../scripts/tests/fixtures/stage1-owner-envelope.json");
        let payload: serde_json::Value = serde_json::from_slice(PAYLOAD).unwrap();
        let envelope: serde_json::Value = serde_json::from_slice(ENVELOPE).unwrap();
        assert_eq!(canonical_json_bytes(&payload).unwrap(), PAYLOAD);
        assert_eq!(canonical_json_bytes(&envelope).unwrap(), ENVELOPE);
        assert_eq!(
            STANDARD
                .decode(envelope["payload_base64"].as_str().unwrap())
                .unwrap(),
            PAYLOAD
        );
        assert_eq!(payload["artifacts"].as_array().unwrap().len(), 5);
        assert_eq!(payload["model_transfer"], "blocked");
    }

    #[test]
    fn authorization_builder_rejects_unprovisioned_or_changed_signing_key() {
        const BINDING: &[u8] =
            include_bytes!("../../../scripts/tests/fixtures/stage1-owner-binding-frozen.json");
        let binding = crate::owner_authorization::binding::parse_frozen_binding(BINDING).unwrap();
        let root = TestRoot::new("issue-key-mismatch");
        let directories =
            open_stage1_directories(&FakeResolver(root.path().to_path_buf())).unwrap();
        let context = || IssueContext {
            binding: &binding,
            directories: &directories,
            now_unix: 1_795_000_000,
            generation: [0x11; 16],
            nonce: [0x22; 32],
        };
        let (backend, calls) = fake_backend(Ok(proof(0x11)), Ok(key(0x11)), Ok(proof(0x22)));
        let record = bootstrap_record_bytes(&key(0x11)).unwrap();
        let anchor = crate::owner_authorization::binding::parse_bootstrap_record(&record).unwrap();
        assert_eq!(
            build_and_sign_authorization(context(), &backend, &anchor),
            Err(OwnerAuthorizationError::KeyMismatch)
        );
        assert_eq!(calls.lock().unwrap().len(), 1);
        let (unused, unused_calls) = fake_backend(Ok(proof(0x11)), Ok(key(0x11)), Ok(proof(0x11)));
        assert_eq!(
            build_and_sign_authorization(context(), &unused, &PublicKeyAnchor::Unprovisioned),
            Err(OwnerAuthorizationError::Binding)
        );
        assert!(unused_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn bounded_reader_never_consumes_more_than_the_limit() {
        assert_eq!(read_bounded(&mut b"abc".as_slice(), 3).unwrap(), b"abc");
        assert_eq!(
            read_bounded(&mut b"abcd".as_slice(), 3),
            Err(OwnerAuthorizationError::CanonicalStorage)
        );
    }

    #[test]
    fn bundled_stage1_paths_are_rooted_at_one_exact_resource_directory() {
        let paths = bundled_stage1_paths(Path::new(
            "/Applications/J.A.R.V.I.S.app/Contents/MacOS/jarvis-dashboard",
        ))
        .unwrap();
        assert_eq!(
            paths.root,
            PathBuf::from("/Applications/J.A.R.V.I.S.app/Contents/Resources/jarvis/stage1")
        );
        assert_eq!(paths.script, paths.root.join("prebuilt_runtime_stage1.py"));
        assert_eq!(paths.binding, paths.root.join("stage1-owner-binding.json"));
        assert_eq!(
            paths.public_key,
            paths.root.join("stage1-owner-public-key.json")
        );
        assert!(bundled_stage1_paths(Path::new("/tmp/jarvis-dashboard")).is_err());
    }

    #[test]
    fn public_errors_expose_only_fixed_codes() {
        for (error, code) in [
            (OwnerAuthorizationError::Bundle, "bundle"),
            (OwnerAuthorizationError::Binding, "binding"),
            (
                OwnerAuthorizationError::SecureEnclaveUnavailable,
                "secure-enclave-unavailable",
            ),
            (OwnerAuthorizationError::KeyNotFound, "key-not-found"),
            (OwnerAuthorizationError::KeyMismatch, "key-mismatch"),
            (OwnerAuthorizationError::UserCancelled, "user-cancelled"),
            (OwnerAuthorizationError::Authentication, "authentication"),
            (OwnerAuthorizationError::Signature, "signature"),
            (OwnerAuthorizationError::Expired, "expired"),
            (
                OwnerAuthorizationError::CanonicalStorage,
                "canonical-storage",
            ),
            (OwnerAuthorizationError::AlreadyConsumed, "already-consumed"),
            (OwnerAuthorizationError::Recovery, "recovery"),
            (OwnerAuthorizationError::PythonLaunch, "python-launch"),
            (OwnerAuthorizationError::Stage1Failed, "stage1-failed"),
            (OwnerAuthorizationError::DeleteRefused, "delete-refused"),
            (
                OwnerAuthorizationError::DeleteIncomplete,
                "delete-incomplete",
            ),
        ] {
            assert_eq!(error.code(), code);
        }
    }
}
