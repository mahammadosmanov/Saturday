use crate::owner_authorization::binding::{parse_bootstrap_record, PublicKeyAnchor};
use crate::owner_authorization::canonical::canonical_json_bytes;
use crate::owner_authorization::issuer::OwnerAuthorizationError;
use crate::owner_authorization::paths::Stage1Directories;
use crate::platform::owner_authorization::{OwnerKeyBackend, OwnerKeyError};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, PermissionsExt};

pub const DELETE_CHALLENGE_DOMAIN: &[u8] = b"SATURDAY-STAGE1-OWNER-AUTHORITY-DELETE-V1\0";
pub const DELETE_PROMPT: &str = "Confirm deletion of the Saturday Stage 1 owner authorization key.";
const BOOTSTRAP_RECORD_NAME: &str = "owner-public-key.json";
const LEDGER_LOCK_NAME: &str = ".saturday-stage1.lock";
const LEDGER_SCHEMA: &str = "saturday.local-provider.stage1-ledger.v2";
const MAX_BOOTSTRAP_BYTES: usize = 4096;
const MAX_LEDGER_BYTES: usize = 64 * 1024;
const MAX_RECEIPT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerStateSummary {
    pub active_or_prepared_generations: u64,
    pub committed_or_published_receipts: u64,
    pub recovery_complete: bool,
    pub valid: bool,
}

pub trait LedgerStateGuard {
    fn summary(&self) -> LedgerStateSummary;
}

pub trait LedgerStateReader {
    type Guard: LedgerStateGuard;

    fn lock_recover_and_summarize(
        &self,
        directories: &Stage1Directories,
    ) -> Result<Self::Guard, OwnerAuthorizationError>;
}

pub trait BootstrapRecordStore {
    fn verify_matches(
        &self,
        directories: &Stage1Directories,
        anchor: &PublicKeyAnchor,
    ) -> Result<(), OwnerAuthorizationError>;

    fn remove_matches(
        &self,
        directories: &Stage1Directories,
        anchor: &PublicKeyAnchor,
    ) -> Result<(), OwnerAuthorizationError>;
}

pub fn delete_owner_key(
    directories: &Stage1Directories,
    key_backend: &impl OwnerKeyBackend,
    anchor: &PublicKeyAnchor,
    ledger_reader: &impl LedgerStateReader,
    bootstrap_store: &impl BootstrapRecordStore,
    _nonce: [u8; 32],
) -> Result<(), OwnerAuthorizationError> {
    let (expected_key_id, expected_public_key) = match anchor {
        PublicKeyAnchor::Pinned {
            key_id,
            public_key_x963,
        } => (key_id, public_key_x963),
        PublicKeyAnchor::Unprovisioned => return Err(OwnerAuthorizationError::Binding),
    };
    let current_public_key = key_backend.copy_public_key().map_err(map_key_error)?;
    if current_public_key.as_slice() != expected_public_key
        || Sha256::digest(&current_public_key).as_slice() != expected_key_id
    {
        return Err(OwnerAuthorizationError::KeyMismatch);
    }
    bootstrap_store.verify_matches(directories, anchor)?;

    let guard = ledger_reader.lock_recover_and_summarize(directories)?;
    let summary = guard.summary();
    if !summary.recovery_complete {
        return Err(OwnerAuthorizationError::DeleteRefused);
    }
    if !summary.valid || summary.active_or_prepared_generations != 0 {
        return Err(OwnerAuthorizationError::DeleteRefused);
    }

    let summary_sha256 = ledger_summary_sha256(summary)?;
    let mut challenge = Vec::with_capacity(DELETE_CHALLENGE_DOMAIN.len() + 64);
    challenge.extend_from_slice(DELETE_CHALLENGE_DOMAIN);
    challenge.extend_from_slice(expected_key_id);
    challenge.extend_from_slice(&summary_sha256);
    key_backend
        .delete_after_proof(&challenge, DELETE_PROMPT)
        .map_err(map_key_error)?;
    if !matches!(
        key_backend.copy_public_key(),
        Err(OwnerKeyError::KeyNotFound)
    ) {
        return Err(OwnerAuthorizationError::DeleteRefused);
    }
    bootstrap_store
        .remove_matches(directories, anchor)
        .map_err(|_| OwnerAuthorizationError::DeleteIncomplete)
}

fn ledger_summary_sha256(summary: LedgerStateSummary) -> Result<[u8; 32], OwnerAuthorizationError> {
    let bytes = canonical_json_bytes(&serde_json::json!({
        "active_or_prepared_generations": summary.active_or_prepared_generations,
        "committed_or_published_receipts": summary.committed_or_published_receipts,
        "recovery_complete": summary.recovery_complete,
        "schema": "saturday.local-provider.stage1-owner-deletion-ledger-summary.v1",
        "valid": summary.valid,
    }))
    .map_err(|_| OwnerAuthorizationError::Recovery)?;
    Ok(Sha256::digest(bytes).into())
}

fn map_key_error(error: OwnerKeyError) -> OwnerAuthorizationError {
    match error {
        OwnerKeyError::SecureEnclaveUnavailable => {
            OwnerAuthorizationError::SecureEnclaveUnavailable
        }
        OwnerKeyError::KeyNotFound => OwnerAuthorizationError::KeyNotFound,
        OwnerKeyError::DuplicateKey => OwnerAuthorizationError::CanonicalStorage,
        OwnerKeyError::UserCancelled => OwnerAuthorizationError::UserCancelled,
        OwnerKeyError::AuthenticationFailed => OwnerAuthorizationError::Authentication,
        OwnerKeyError::InvalidPublicKey => OwnerAuthorizationError::KeyMismatch,
        OwnerKeyError::InvalidSignature => OwnerAuthorizationError::Signature,
        OwnerKeyError::System => OwnerAuthorizationError::Authentication,
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeBootstrapRecordStore;

impl BootstrapRecordStore for NativeBootstrapRecordStore {
    fn verify_matches(
        &self,
        directories: &Stage1Directories,
        anchor: &PublicKeyAnchor,
    ) -> Result<(), OwnerAuthorizationError> {
        verify_bootstrap_record(directories, anchor)
    }

    fn remove_matches(
        &self,
        directories: &Stage1Directories,
        anchor: &PublicKeyAnchor,
    ) -> Result<(), OwnerAuthorizationError> {
        verify_bootstrap_record(directories, anchor)?;
        let name = CString::new(BOOTSTRAP_RECORD_NAME)
            .map_err(|_| OwnerAuthorizationError::CanonicalStorage)?;
        if unsafe { libc::unlinkat(directories.bootstrap.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(OwnerAuthorizationError::CanonicalStorage);
        }
        if unsafe { libc::fsync(directories.bootstrap.as_raw_fd()) } != 0 {
            return Err(OwnerAuthorizationError::CanonicalStorage);
        }
        Ok(())
    }
}

fn verify_bootstrap_record(
    directories: &Stage1Directories,
    expected: &PublicKeyAnchor,
) -> Result<(), OwnerAuthorizationError> {
    let bytes = read_regular_at(
        directories.bootstrap.as_fd(),
        BOOTSTRAP_RECORD_NAME,
        MAX_BOOTSTRAP_BYTES,
    )?;
    let observed = parse_bootstrap_record(&bytes).map_err(|_| OwnerAuthorizationError::Binding)?;
    if &observed == expected {
        Ok(())
    } else {
        Err(OwnerAuthorizationError::KeyMismatch)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeLedgerStateReader;

pub struct NativeLedgerStateGuard {
    _lock: File,
    summary: LedgerStateSummary,
}

impl LedgerStateGuard for NativeLedgerStateGuard {
    fn summary(&self) -> LedgerStateSummary {
        self.summary
    }
}

impl LedgerStateReader for NativeLedgerStateReader {
    type Guard = NativeLedgerStateGuard;

    fn lock_recover_and_summarize(
        &self,
        directories: &Stage1Directories,
    ) -> Result<Self::Guard, OwnerAuthorizationError> {
        let lock = open_ledger_lock(directories)?;
        recover_registered_workspaces(directories)?;
        let summary = summarize_locked_state(directories)?;
        Ok(NativeLedgerStateGuard {
            _lock: lock,
            summary,
        })
    }
}

fn open_ledger_lock(directories: &Stage1Directories) -> Result<File, OwnerAuthorizationError> {
    let name = CString::new(LEDGER_LOCK_NAME).map_err(|_| OwnerAuthorizationError::Recovery)?;
    let descriptor = unsafe {
        libc::openat(
            directories.ledger.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(OwnerAuthorizationError::Recovery);
    }
    // SAFETY: openat returned one newly owned descriptor.
    let file = unsafe { File::from_raw_fd(descriptor) };
    let metadata = file
        .metadata()
        .map_err(|_| OwnerAuthorizationError::Recovery)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(OwnerAuthorizationError::Recovery);
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(OwnerAuthorizationError::Recovery);
    }
    Ok(file)
}

#[derive(Debug)]
struct ReceiptExpectation {
    sha256: [u8; 32],
    required: bool,
}

#[derive(Debug)]
struct RecoveryMarker {
    name: String,
    generation: String,
    marker: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug)]
struct WorkspaceRegistration {
    parent_device: u64,
    parent_inode: u64,
    parent_uid: u32,
    name: String,
    device: u64,
    inode: u64,
    uid: u32,
}

fn recover_registered_workspaces(
    directories: &Stage1Directories,
) -> Result<(), OwnerAuthorizationError> {
    let mut markers = Vec::new();
    for name in directory_names(directories.ledger.as_fd())? {
        if name == LEDGER_LOCK_NAME {
            continue;
        }
        let generation = marker_generation(&name)
            .ok_or(OwnerAuthorizationError::Recovery)?
            .to_owned();
        let bytes = read_regular_at(directories.ledger.as_fd(), &name, MAX_LEDGER_BYTES)?;
        let marker = canonical_object(&bytes)?;
        validate_marker_shape(&marker, &generation)?;
        if let Some(workspace) = marker.get("workspace").filter(|value| !value.is_null()) {
            if marker.get("state").and_then(serde_json::Value::as_str) != Some("authorized") {
                return Err(OwnerAuthorizationError::Recovery);
            }
            validate_empty_receipt_state(&marker)?;
            parse_workspace(workspace, &generation)?;
        }
        markers.push(RecoveryMarker {
            name,
            generation,
            marker,
        });
    }

    for mut record in markers {
        let Some(workspace_value) = record
            .marker
            .get("workspace")
            .filter(|value| !value.is_null())
            .cloned()
        else {
            continue;
        };
        let workspace = parse_workspace(&workspace_value, &record.generation)?;
        if workspace.parent_device != directories.workspace_parent_identity.device
            || workspace.parent_inode != directories.workspace_parent_identity.inode
            || workspace.parent_uid != directories.workspace_parent_identity.uid
        {
            return Err(OwnerAuthorizationError::Recovery);
        }
        remove_registered_workspace(directories, &record.generation, &record.marker, &workspace)?;
        record
            .marker
            .insert("workspace".to_owned(), serde_json::Value::Null);
        record.marker.insert(
            "state".to_owned(),
            serde_json::Value::String("recovered".to_owned()),
        );
        replace_ledger_marker(directories, &record.name, &record.marker)?;
    }

    if !directory_names(directories.workspace_parent.as_fd())?.is_empty() {
        return Err(OwnerAuthorizationError::Recovery);
    }
    Ok(())
}

fn parse_workspace(
    value: &serde_json::Value,
    generation: &str,
) -> Result<WorkspaceRegistration, OwnerAuthorizationError> {
    validate_workspace(value, generation)?;
    let object = value.as_object().ok_or(OwnerAuthorizationError::Recovery)?;
    let u64_field = |name: &str| {
        object
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .ok_or(OwnerAuthorizationError::Recovery)
    };
    Ok(WorkspaceRegistration {
        parent_device: u64_field("parent_device")?,
        parent_inode: u64_field("parent_inode")?,
        parent_uid: u32::try_from(u64_field("parent_uid")?)
            .map_err(|_| OwnerAuthorizationError::Recovery)?,
        name: object
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or(OwnerAuthorizationError::Recovery)?
            .to_owned(),
        device: u64_field("device")?,
        inode: u64_field("inode")?,
        uid: u32::try_from(u64_field("uid")?).map_err(|_| OwnerAuthorizationError::Recovery)?,
    })
}

fn remove_registered_workspace(
    directories: &Stage1Directories,
    generation: &str,
    marker: &serde_json::Map<String, serde_json::Value>,
    workspace: &WorkspaceRegistration,
) -> Result<(), OwnerAuthorizationError> {
    let Some(before) = stat_at(directories.workspace_parent.as_fd(), &workspace.name)? else {
        return Ok(());
    };
    if before.st_mode & libc::S_IFMT != libc::S_IFDIR
        || before.st_mode & 0o777 != 0o700
        || before.st_uid != unsafe { libc::geteuid() }
        || before.st_dev as u64 != workspace.device
        || before.st_ino != workspace.inode
        || before.st_uid != workspace.uid
    {
        return Err(OwnerAuthorizationError::Recovery);
    }
    let root = open_directory_at(directories.workspace_parent.as_fd(), &workspace.name)?;
    let opened = root
        .metadata()
        .map_err(|_| OwnerAuthorizationError::Recovery)?;
    if opened.dev() != workspace.device
        || opened.ino() != workspace.inode
        || opened.uid() != workspace.uid
    {
        return Err(OwnerAuthorizationError::Recovery);
    }
    validate_workspace_owner(root.as_fd(), generation, marker, workspace)?;
    clear_directory(root.as_fd())?;
    if !directory_names(root.as_fd())?.is_empty() {
        return Err(OwnerAuthorizationError::Recovery);
    }
    drop(root);
    let current = stat_at(directories.workspace_parent.as_fd(), &workspace.name)?
        .ok_or(OwnerAuthorizationError::Recovery)?;
    if current.st_dev != before.st_dev || current.st_ino != before.st_ino {
        return Err(OwnerAuthorizationError::Recovery);
    }
    let name =
        CString::new(workspace.name.as_str()).map_err(|_| OwnerAuthorizationError::Recovery)?;
    if unsafe {
        libc::unlinkat(
            directories.workspace_parent.as_raw_fd(),
            name.as_ptr(),
            libc::AT_REMOVEDIR,
        )
    } != 0
        || unsafe { libc::fsync(directories.workspace_parent.as_raw_fd()) } != 0
    {
        return Err(OwnerAuthorizationError::Recovery);
    }
    Ok(())
}

fn validate_workspace_owner(
    workspace_fd: std::os::fd::BorrowedFd<'_>,
    generation: &str,
    marker: &serde_json::Map<String, serde_json::Value>,
    workspace: &WorkspaceRegistration,
) -> Result<(), OwnerAuthorizationError> {
    let bytes = read_regular_at(workspace_fd, ".saturday-stage1-owner.json", 4096)?;
    let owner = canonical_object(&bytes)?;
    const KEYS: [&str; 10] = [
        "schema",
        "generation",
        "authorization_sha256",
        "parent_device",
        "parent_inode",
        "parent_uid",
        "name",
        "device",
        "inode",
        "uid",
    ];
    if owner.len() != KEYS.len() || KEYS.iter().any(|key| !owner.contains_key(*key)) {
        return Err(OwnerAuthorizationError::Recovery);
    }
    let expected_authorization = marker
        .get("authorization_sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or(OwnerAuthorizationError::Recovery)?;
    if owner.get("schema").and_then(serde_json::Value::as_str)
        != Some("saturday.local-provider.stage1-workspace.v2")
        || owner.get("generation").and_then(serde_json::Value::as_str) != Some(generation)
        || owner
            .get("authorization_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(expected_authorization)
        || owner.get("name").and_then(serde_json::Value::as_str) != Some(workspace.name.as_str())
    {
        return Err(OwnerAuthorizationError::Recovery);
    }
    for (field, expected) in [
        ("parent_device", workspace.parent_device),
        ("parent_inode", workspace.parent_inode),
        ("parent_uid", u64::from(workspace.parent_uid)),
        ("device", workspace.device),
        ("inode", workspace.inode),
        ("uid", u64::from(workspace.uid)),
    ] {
        if owner.get(field).and_then(serde_json::Value::as_u64) != Some(expected) {
            return Err(OwnerAuthorizationError::Recovery);
        }
    }
    Ok(())
}

fn clear_directory(directory: std::os::fd::BorrowedFd<'_>) -> Result<(), OwnerAuthorizationError> {
    for name in directory_names(directory)? {
        let before = stat_at(directory, &name)?.ok_or(OwnerAuthorizationError::Recovery)?;
        let encoded = CString::new(name.as_str()).map_err(|_| OwnerAuthorizationError::Recovery)?;
        match before.st_mode & libc::S_IFMT {
            libc::S_IFDIR => {
                let child = open_directory_at(directory, &name)?;
                clear_directory(child.as_fd())?;
                if !directory_names(child.as_fd())?.is_empty() {
                    return Err(OwnerAuthorizationError::Recovery);
                }
                drop(child);
                let after = stat_at(directory, &name)?.ok_or(OwnerAuthorizationError::Recovery)?;
                if before.st_dev != after.st_dev || before.st_ino != after.st_ino {
                    return Err(OwnerAuthorizationError::Recovery);
                }
                if unsafe {
                    libc::unlinkat(directory.as_raw_fd(), encoded.as_ptr(), libc::AT_REMOVEDIR)
                } != 0
                {
                    return Err(OwnerAuthorizationError::Recovery);
                }
            }
            libc::S_IFREG | libc::S_IFLNK => {
                if unsafe { libc::unlinkat(directory.as_raw_fd(), encoded.as_ptr(), 0) } != 0 {
                    return Err(OwnerAuthorizationError::Recovery);
                }
            }
            _ => return Err(OwnerAuthorizationError::Recovery),
        }
    }
    if unsafe { libc::fsync(directory.as_raw_fd()) } != 0 {
        return Err(OwnerAuthorizationError::Recovery);
    }
    Ok(())
}

fn open_directory_at(
    parent: std::os::fd::BorrowedFd<'_>,
    name: &str,
) -> Result<File, OwnerAuthorizationError> {
    let name = CString::new(name).map_err(|_| OwnerAuthorizationError::Recovery)?;
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(OwnerAuthorizationError::Recovery);
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn stat_at(
    parent: std::os::fd::BorrowedFd<'_>,
    name: &str,
) -> Result<Option<libc::stat>, OwnerAuthorizationError> {
    let name = CString::new(name).map_err(|_| OwnerAuthorizationError::Recovery)?;
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } == 0
    {
        return Ok(Some(unsafe { metadata.assume_init() }));
    }
    if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
        Ok(None)
    } else {
        Err(OwnerAuthorizationError::Recovery)
    }
}

fn replace_ledger_marker(
    directories: &Stage1Directories,
    name: &str,
    marker: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), OwnerAuthorizationError> {
    let payload = serde_json::to_vec(&serde_json::Value::Object(marker.clone()))
        .map_err(|_| OwnerAuthorizationError::Recovery)?;
    canonical_object(&payload)?;
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| OwnerAuthorizationError::Recovery)?;
    let temporary = format!(".{name}.recovery-{}", lower_hex(&random));
    let temporary_c =
        CString::new(temporary.as_str()).map_err(|_| OwnerAuthorizationError::Recovery)?;
    let descriptor = unsafe {
        libc::openat(
            directories.ledger.as_raw_fd(),
            temporary_c.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(OwnerAuthorizationError::Recovery);
    }
    let mut file = unsafe { File::from_raw_fd(descriptor) };
    let write_result = file
        .write_all(&payload)
        .and_then(|()| file.sync_all())
        .map_err(|_| OwnerAuthorizationError::Recovery);
    drop(file);
    if let Err(error) = write_result {
        unsafe {
            libc::unlinkat(directories.ledger.as_raw_fd(), temporary_c.as_ptr(), 0);
        }
        return Err(error);
    }
    let final_c = CString::new(name).map_err(|_| OwnerAuthorizationError::Recovery)?;
    if unsafe {
        libc::renameat(
            directories.ledger.as_raw_fd(),
            temporary_c.as_ptr(),
            directories.ledger.as_raw_fd(),
            final_c.as_ptr(),
        )
    } != 0
        || unsafe { libc::fsync(directories.ledger.as_raw_fd()) } != 0
    {
        unsafe {
            libc::unlinkat(directories.ledger.as_raw_fd(), temporary_c.as_ptr(), 0);
        }
        return Err(OwnerAuthorizationError::Recovery);
    }
    Ok(())
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn summarize_locked_state(
    directories: &Stage1Directories,
) -> Result<LedgerStateSummary, OwnerAuthorizationError> {
    let workspace_names = directory_names(directories.workspace_parent.as_fd())?;
    let mut recovery_complete = workspace_names.is_empty();
    let mut active_or_prepared_generations = 0_u64;
    let mut committed_or_published_receipts = 0_u64;
    let mut expected_receipts = HashMap::new();
    let mut generations = BTreeSet::new();

    for name in directory_names(directories.ledger.as_fd())? {
        if name == LEDGER_LOCK_NAME {
            continue;
        }
        let Some(generation) = marker_generation(&name) else {
            return Err(OwnerAuthorizationError::Recovery);
        };
        if !generations.insert(generation.to_owned()) {
            return Err(OwnerAuthorizationError::Recovery);
        }
        let bytes = read_regular_at(directories.ledger.as_fd(), &name, MAX_LEDGER_BYTES)?;
        let marker = canonical_object(&bytes)?;
        validate_marker_shape(&marker, generation)?;
        let state = marker
            .get("state")
            .and_then(serde_json::Value::as_str)
            .ok_or(OwnerAuthorizationError::Recovery)?;
        let workspace = marker
            .get("workspace")
            .ok_or(OwnerAuthorizationError::Recovery)?;
        if !workspace.is_null() {
            validate_workspace(workspace, generation)?;
            recovery_complete = false;
        }
        match state {
            "authorized" => {
                validate_empty_receipt_state(&marker)?;
                active_or_prepared_generations = active_or_prepared_generations
                    .checked_add(1)
                    .ok_or(OwnerAuthorizationError::Recovery)?;
            }
            "recovered" | "failed_cleaned" => {
                if !workspace.is_null() {
                    return Err(OwnerAuthorizationError::Recovery);
                }
                validate_empty_receipt_state(&marker)?;
            }
            "cleaned" | "published" => {
                if !workspace.is_null() {
                    return Err(OwnerAuthorizationError::Recovery);
                }
                let (receipt_name, receipt_sha256) =
                    validate_committed_receipt_state(&marker, directories)?;
                if expected_receipts
                    .insert(
                        receipt_name,
                        ReceiptExpectation {
                            sha256: receipt_sha256,
                            required: state == "published",
                        },
                    )
                    .is_some()
                {
                    return Err(OwnerAuthorizationError::Recovery);
                }
                committed_or_published_receipts = committed_or_published_receipts
                    .checked_add(1)
                    .ok_or(OwnerAuthorizationError::Recovery)?;
            }
            _ => return Err(OwnerAuthorizationError::Recovery),
        }
    }

    validate_receipt_directory(directories, &expected_receipts)?;
    Ok(LedgerStateSummary {
        active_or_prepared_generations,
        committed_or_published_receipts,
        recovery_complete,
        valid: true,
    })
}

fn marker_generation(name: &str) -> Option<&str> {
    name.strip_prefix(".saturday-stage1-")
        .and_then(|value| value.strip_suffix(".json"))
        .filter(|value| is_lower_hex(value, 16))
}

fn validate_marker_shape(
    marker: &serde_json::Map<String, serde_json::Value>,
    generation: &str,
) -> Result<(), OwnerAuthorizationError> {
    const KEYS: [&str; 15] = [
        "schema",
        "state",
        "generation",
        "authorization_sha256",
        "binding_sha256",
        "key_id",
        "candidate_id",
        "candidate_sha256",
        "review_aggregate_sha256",
        "review_manifest_sha256",
        "run_id",
        "receipt_sha256",
        "cleanup_sha256",
        "receipt_registration",
        "workspace",
    ];
    if marker.len() != KEYS.len() || KEYS.iter().any(|key| !marker.contains_key(*key)) {
        return Err(OwnerAuthorizationError::Recovery);
    }
    if marker.get("schema").and_then(serde_json::Value::as_str) != Some(LEDGER_SCHEMA)
        || marker.get("generation").and_then(serde_json::Value::as_str) != Some(generation)
        || marker
            .get("candidate_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty() && printable_ascii(value))
            .is_none()
    {
        return Err(OwnerAuthorizationError::Recovery);
    }
    for field in [
        "authorization_sha256",
        "binding_sha256",
        "key_id",
        "candidate_sha256",
        "review_aggregate_sha256",
        "review_manifest_sha256",
    ] {
        if marker
            .get(field)
            .and_then(serde_json::Value::as_str)
            .filter(|value| is_lower_hex(value, 32))
            .is_none()
        {
            return Err(OwnerAuthorizationError::Recovery);
        }
    }
    Ok(())
}

fn validate_empty_receipt_state(
    marker: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), OwnerAuthorizationError> {
    for field in [
        "run_id",
        "receipt_sha256",
        "cleanup_sha256",
        "receipt_registration",
    ] {
        if !marker
            .get(field)
            .ok_or(OwnerAuthorizationError::Recovery)?
            .is_null()
        {
            return Err(OwnerAuthorizationError::Recovery);
        }
    }
    Ok(())
}

fn validate_workspace(
    value: &serde_json::Value,
    generation: &str,
) -> Result<(), OwnerAuthorizationError> {
    const KEYS: [&str; 7] = [
        "parent_device",
        "parent_inode",
        "parent_uid",
        "name",
        "device",
        "inode",
        "uid",
    ];
    let object = value.as_object().ok_or(OwnerAuthorizationError::Recovery)?;
    if object.len() != KEYS.len() || KEYS.iter().any(|key| !object.contains_key(*key)) {
        return Err(OwnerAuthorizationError::Recovery);
    }
    for key in KEYS.into_iter().filter(|key| *key != "name") {
        if object
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .is_none()
        {
            return Err(OwnerAuthorizationError::Recovery);
        }
    }
    if object.get("name").and_then(serde_json::Value::as_str)
        != Some(&format!("saturday-prebuilt-{generation}"))
    {
        return Err(OwnerAuthorizationError::Recovery);
    }
    Ok(())
}

fn validate_committed_receipt_state(
    marker: &serde_json::Map<String, serde_json::Value>,
    directories: &Stage1Directories,
) -> Result<(String, [u8; 32]), OwnerAuthorizationError> {
    let run_id = marker
        .get("run_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty() && printable_ascii(value))
        .ok_or(OwnerAuthorizationError::Recovery)?;
    let _ = run_id;
    let receipt_sha256 = decode_hex_32(
        marker
            .get("receipt_sha256")
            .and_then(serde_json::Value::as_str)
            .ok_or(OwnerAuthorizationError::Recovery)?,
    )?;
    for field in ["cleanup_sha256"] {
        if marker
            .get(field)
            .and_then(serde_json::Value::as_str)
            .filter(|value| is_lower_hex(value, 32))
            .is_none()
        {
            return Err(OwnerAuthorizationError::Recovery);
        }
    }
    let registration = marker
        .get("receipt_registration")
        .and_then(serde_json::Value::as_object)
        .ok_or(OwnerAuthorizationError::Recovery)?;
    const KEYS: [&str; 4] = ["parent_device", "parent_inode", "parent_uid", "name"];
    if registration.len() != KEYS.len() || KEYS.iter().any(|key| !registration.contains_key(*key)) {
        return Err(OwnerAuthorizationError::Recovery);
    }
    let identity = [
        directories.receipt_parent_identity.device,
        directories.receipt_parent_identity.inode,
        u64::from(directories.receipt_parent_identity.uid),
    ];
    for (field, expected) in ["parent_device", "parent_inode", "parent_uid"]
        .into_iter()
        .zip(identity)
    {
        if registration.get(field).and_then(serde_json::Value::as_u64) != Some(expected) {
            return Err(OwnerAuthorizationError::Recovery);
        }
    }
    let name = registration
        .get("name")
        .and_then(serde_json::Value::as_str)
        .filter(|value| receipt_generation(value).is_some())
        .ok_or(OwnerAuthorizationError::Recovery)?;
    Ok((name.to_owned(), receipt_sha256))
}

fn receipt_generation(name: &str) -> Option<&str> {
    name.strip_prefix("stage1-")
        .and_then(|value| value.strip_suffix(".json"))
        .filter(|value| is_lower_hex(value, 16))
}

fn validate_receipt_directory(
    directories: &Stage1Directories,
    expected: &HashMap<String, ReceiptExpectation>,
) -> Result<(), OwnerAuthorizationError> {
    let names = directory_names(directories.receipt_parent.as_fd())?;
    for name in &names {
        let expectation = expected
            .get(name)
            .ok_or(OwnerAuthorizationError::Recovery)?;
        let bytes = read_regular_at(directories.receipt_parent.as_fd(), name, MAX_RECEIPT_BYTES)?;
        if Sha256::digest(&bytes).as_slice() != expectation.sha256 {
            return Err(OwnerAuthorizationError::Recovery);
        }
        canonical_object(&bytes)?;
    }
    for (name, expectation) in expected {
        if expectation.required && !names.contains(name) {
            return Err(OwnerAuthorizationError::Recovery);
        }
    }
    Ok(())
}

fn canonical_object(
    bytes: &[u8],
) -> Result<serde_json::Map<String, serde_json::Value>, OwnerAuthorizationError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| OwnerAuthorizationError::Recovery)?;
    validate_json_value(&value)?;
    let object = value.as_object().ok_or(OwnerAuthorizationError::Recovery)?;
    if serde_json::to_vec(&value).map_err(|_| OwnerAuthorizationError::Recovery)? != bytes {
        return Err(OwnerAuthorizationError::Recovery);
    }
    Ok(object.clone())
}

fn validate_json_value(value: &serde_json::Value) -> Result<(), OwnerAuthorizationError> {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) => Ok(()),
        serde_json::Value::Number(number) if number.as_i64().is_some() => Ok(()),
        serde_json::Value::String(value) if printable_ascii(value) => Ok(()),
        serde_json::Value::Array(values) => {
            for value in values {
                validate_json_value(value)?;
            }
            Ok(())
        }
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if !printable_ascii(key) {
                    return Err(OwnerAuthorizationError::Recovery);
                }
                validate_json_value(value)?;
            }
            Ok(())
        }
        _ => Err(OwnerAuthorizationError::Recovery),
    }
}

fn directory_names(
    directory: std::os::fd::BorrowedFd<'_>,
) -> Result<Vec<String>, OwnerAuthorizationError> {
    let duplicated = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 8) };
    if duplicated < 0 {
        return Err(OwnerAuthorizationError::Recovery);
    }
    // SAFETY: fcntl returned one newly owned descriptor, transferred to fdopendir.
    let duplicate = unsafe { OwnedFd::from_raw_fd(duplicated) };
    let raw = duplicate.into_raw_fd();
    let directory = unsafe { libc::fdopendir(raw) };
    if directory.is_null() {
        unsafe {
            libc::close(raw);
        }
        return Err(OwnerAuthorizationError::Recovery);
    }
    let mut names = Vec::new();
    let error = loop {
        unsafe {
            *libc::__error() = 0;
        }
        let entry = unsafe { libc::readdir(directory) };
        if entry.is_null() {
            break unsafe { *libc::__error() } != 0;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
            .to_str()
            .map_err(|_| OwnerAuthorizationError::Recovery)?;
        if name == "." || name == ".." {
            continue;
        }
        if !printable_ascii(name) {
            break true;
        }
        names.push(name.to_owned());
    };
    if unsafe { libc::closedir(directory) } != 0 || error {
        return Err(OwnerAuthorizationError::Recovery);
    }
    names.sort();
    Ok(names)
}

fn read_regular_at(
    directory: std::os::fd::BorrowedFd<'_>,
    name: &str,
    maximum_bytes: usize,
) -> Result<Vec<u8>, OwnerAuthorizationError> {
    let name = CString::new(name).map_err(|_| OwnerAuthorizationError::Recovery)?;
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(OwnerAuthorizationError::Recovery);
    }
    // SAFETY: openat returned one newly owned descriptor.
    let mut file = unsafe { File::from_raw_fd(descriptor) };
    let before = file
        .metadata()
        .map_err(|_| OwnerAuthorizationError::Recovery)?;
    if !before.is_file()
        || before.nlink() != 1
        || before.uid() != unsafe { libc::geteuid() }
        || before.permissions().mode() & 0o777 != 0o600
        || before.len() == 0
        || before.len() > maximum_bytes as u64
    {
        return Err(OwnerAuthorizationError::Recovery);
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(maximum_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| OwnerAuthorizationError::Recovery)?;
    let after = file
        .metadata()
        .map_err(|_| OwnerAuthorizationError::Recovery)?;
    if bytes.len() > maximum_bytes
        || before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.len() != after.len()
        || bytes.len() as u64 != before.len()
    {
        return Err(OwnerAuthorizationError::Recovery);
    }
    Ok(bytes)
}

fn decode_hex_32(value: &str) -> Result<[u8; 32], OwnerAuthorizationError> {
    if !is_lower_hex(value, 32) {
        return Err(OwnerAuthorizationError::Recovery);
    }
    let mut output = [0_u8; 32];
    for (index, chunk) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        output[index] = (hex_nibble(chunk[0])? << 4) | hex_nibble(chunk[1])?;
    }
    Ok(output)
}

fn hex_nibble(value: u8) -> Result<u8, OwnerAuthorizationError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(OwnerAuthorizationError::Recovery),
    }
}

fn is_lower_hex(value: &str, bytes: usize) -> bool {
    value.len() == bytes.saturating_mul(2)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn printable_ascii(value: &str) -> bool {
    value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner_authorization::binding::{bootstrap_record_bytes, parse_bootstrap_record};
    use crate::owner_authorization::paths::{
        open_stage1_directories, AccountHomeResolver, PathError,
    };
    use crate::platform::owner_authorization::{KeyProof, OwnerKeyError};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let path = std::env::temp_dir().join(format!(
                "saturday-owner-delete-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Resolver(PathBuf);

    impl AccountHomeResolver for Resolver {
        fn effective_user_home(&self) -> Result<PathBuf, PathError> {
            Ok(self.0.clone())
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum BackendCall {
        Copy,
        Delete,
    }

    #[derive(Debug, Clone, Copy)]
    enum DeleteBehavior {
        SuccessAbsent,
        SuccessStillPresent,
        UserCancelled,
        Failure,
    }

    struct FakeBackend {
        key: Vec<u8>,
        behavior: DeleteBehavior,
        absent: AtomicBool,
        lock_active: Arc<AtomicBool>,
        calls: Mutex<Vec<BackendCall>>,
        challenge: Mutex<Option<Vec<u8>>>,
        prompt: Mutex<Option<String>>,
    }

    impl FakeBackend {
        fn new(key: Vec<u8>, behavior: DeleteBehavior, lock_active: Arc<AtomicBool>) -> Self {
            Self {
                key,
                behavior,
                absent: AtomicBool::new(false),
                lock_active,
                calls: Mutex::new(Vec::new()),
                challenge: Mutex::new(None),
                prompt: Mutex::new(None),
            }
        }
    }

    impl OwnerKeyBackend for FakeBackend {
        fn provision(&self, _challenge: &[u8]) -> Result<KeyProof, OwnerKeyError> {
            Err(OwnerKeyError::System)
        }

        fn copy_public_key(&self) -> Result<Vec<u8>, OwnerKeyError> {
            self.calls.lock().unwrap().push(BackendCall::Copy);
            if self.absent.load(Ordering::SeqCst) {
                Err(OwnerKeyError::KeyNotFound)
            } else {
                Ok(self.key.clone())
            }
        }

        fn sign(&self, _message: &[u8], _prompt: &str) -> Result<KeyProof, OwnerKeyError> {
            Err(OwnerKeyError::System)
        }

        fn delete_after_proof(&self, challenge: &[u8], prompt: &str) -> Result<(), OwnerKeyError> {
            assert!(self.lock_active.load(Ordering::SeqCst));
            self.calls.lock().unwrap().push(BackendCall::Delete);
            *self.challenge.lock().unwrap() = Some(challenge.to_vec());
            *self.prompt.lock().unwrap() = Some(prompt.to_owned());
            match self.behavior {
                DeleteBehavior::SuccessAbsent => {
                    self.absent.store(true, Ordering::SeqCst);
                    Ok(())
                }
                DeleteBehavior::SuccessStillPresent => Ok(()),
                DeleteBehavior::UserCancelled => Err(OwnerKeyError::UserCancelled),
                DeleteBehavior::Failure => Err(OwnerKeyError::System),
            }
        }
    }

    struct FakeGuard {
        summary: LedgerStateSummary,
        active: Arc<AtomicBool>,
    }

    impl LedgerStateGuard for FakeGuard {
        fn summary(&self) -> LedgerStateSummary {
            self.summary
        }
    }

    impl Drop for FakeGuard {
        fn drop(&mut self) {
            self.active.store(false, Ordering::SeqCst);
        }
    }

    struct FakeReader {
        result: Result<LedgerStateSummary, OwnerAuthorizationError>,
        active: Arc<AtomicBool>,
        calls: AtomicU64,
    }

    impl LedgerStateReader for FakeReader {
        type Guard = FakeGuard;

        fn lock_recover_and_summarize(
            &self,
            _directories: &Stage1Directories,
        ) -> Result<Self::Guard, OwnerAuthorizationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let summary = self.result?;
            self.active.store(true, Ordering::SeqCst);
            Ok(FakeGuard {
                summary,
                active: Arc::clone(&self.active),
            })
        }
    }

    struct FakeBootstrapStore {
        verify_result: Result<(), OwnerAuthorizationError>,
        remove_result: Result<(), OwnerAuthorizationError>,
        lock_active: Arc<AtomicBool>,
        verified: AtomicU64,
        removed: AtomicU64,
    }

    impl BootstrapRecordStore for FakeBootstrapStore {
        fn verify_matches(
            &self,
            _directories: &Stage1Directories,
            _anchor: &PublicKeyAnchor,
        ) -> Result<(), OwnerAuthorizationError> {
            self.verified.fetch_add(1, Ordering::SeqCst);
            self.verify_result
        }

        fn remove_matches(
            &self,
            _directories: &Stage1Directories,
            _anchor: &PublicKeyAnchor,
        ) -> Result<(), OwnerAuthorizationError> {
            assert!(self.lock_active.load(Ordering::SeqCst));
            self.removed.fetch_add(1, Ordering::SeqCst);
            self.remove_result
        }
    }

    struct Fixture {
        _root: TestRoot,
        directories: Stage1Directories,
        anchor: PublicKeyAnchor,
        key: Vec<u8>,
        lock_active: Arc<AtomicBool>,
    }

    impl Fixture {
        fn new() -> Self {
            let root = TestRoot::new("fixture");
            let directories = open_stage1_directories(&Resolver(root.0.clone())).unwrap();
            let mut key = vec![0x31; 65];
            key[0] = 0x04;
            let anchor = parse_bootstrap_record(&bootstrap_record_bytes(&key).unwrap()).unwrap();
            Self {
                _root: root,
                directories,
                anchor,
                key,
                lock_active: Arc::new(AtomicBool::new(false)),
            }
        }

        fn reader(
            &self,
            result: Result<LedgerStateSummary, OwnerAuthorizationError>,
        ) -> FakeReader {
            FakeReader {
                result,
                active: Arc::clone(&self.lock_active),
                calls: AtomicU64::new(0),
            }
        }

        fn store(&self, remove_result: Result<(), OwnerAuthorizationError>) -> FakeBootstrapStore {
            FakeBootstrapStore {
                verify_result: Ok(()),
                remove_result,
                lock_active: Arc::clone(&self.lock_active),
                verified: AtomicU64::new(0),
                removed: AtomicU64::new(0),
            }
        }
    }

    fn quiescent() -> LedgerStateSummary {
        LedgerStateSummary {
            active_or_prepared_generations: 0,
            committed_or_published_receipts: 0,
            recovery_complete: true,
            valid: true,
        }
    }

    #[test]
    fn native_reader_holds_the_same_exclusive_lock_for_a_quiescent_summary() {
        let fixture = Fixture::new();
        let guard = NativeLedgerStateReader
            .lock_recover_and_summarize(&fixture.directories)
            .unwrap();
        assert_eq!(guard.summary(), quiescent());
        assert!(matches!(
            NativeLedgerStateReader.lock_recover_and_summarize(&fixture.directories),
            Err(OwnerAuthorizationError::Recovery)
        ));
        drop(guard);
        assert_eq!(
            NativeLedgerStateReader
                .lock_recover_and_summarize(&fixture.directories)
                .unwrap()
                .summary(),
            quiescent()
        );
    }

    #[test]
    fn native_reader_recovers_registered_workspace_before_returning_summary() {
        let fixture = Fixture::new();
        let generation = "b".repeat(32);
        let canonical_root = fixture
            ._root
            .0
            .join("Library/Application Support/dev.rohtolos.saturday/Stage1OwnerAuthorization");
        let workspace_name = format!("saturday-prebuilt-{generation}");
        let workspace = canonical_root.join("workspace").join(&workspace_name);
        fs::create_dir(&workspace).unwrap();
        fs::set_permissions(
            &workspace,
            <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .unwrap();
        let nested = workspace.join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("payload.bin"), b"owned").unwrap();
        let metadata = fs::metadata(&workspace).unwrap();
        let registration = serde_json::json!({
            "device": metadata.dev(),
            "inode": metadata.ino(),
            "name": workspace_name,
            "parent_device": fixture.directories.workspace_parent_identity.device,
            "parent_inode": fixture.directories.workspace_parent_identity.inode,
            "parent_uid": fixture.directories.workspace_parent_identity.uid,
            "uid": metadata.uid(),
        });
        let authorization_sha256 = "1".repeat(64);
        let owner = serde_json::json!({
            "authorization_sha256": authorization_sha256,
            "device": metadata.dev(),
            "generation": generation,
            "inode": metadata.ino(),
            "name": registration["name"],
            "parent_device": fixture.directories.workspace_parent_identity.device,
            "parent_inode": fixture.directories.workspace_parent_identity.inode,
            "parent_uid": fixture.directories.workspace_parent_identity.uid,
            "schema": "saturday.local-provider.stage1-workspace.v2",
            "uid": metadata.uid(),
        });
        let owner_path = workspace.join(".saturday-stage1-owner.json");
        fs::write(&owner_path, serde_json::to_vec(&owner).unwrap()).unwrap();
        fs::set_permissions(
            &owner_path,
            <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .unwrap();
        let marker = serde_json::json!({
            "authorization_sha256": authorization_sha256,
            "binding_sha256": "2".repeat(64),
            "candidate_id": "fixture",
            "candidate_sha256": "3".repeat(64),
            "cleanup_sha256": null,
            "generation": generation,
            "key_id": "4".repeat(64),
            "receipt_registration": null,
            "receipt_sha256": null,
            "review_aggregate_sha256": "5".repeat(64),
            "review_manifest_sha256": "6".repeat(64),
            "run_id": null,
            "schema": LEDGER_SCHEMA,
            "state": "authorized",
            "workspace": registration,
        });
        let marker_path = canonical_root
            .join("ledger")
            .join(format!(".saturday-stage1-{generation}.json"));
        fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        fs::set_permissions(
            &marker_path,
            <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .unwrap();

        let guard = NativeLedgerStateReader
            .lock_recover_and_summarize(&fixture.directories)
            .unwrap();
        assert_eq!(guard.summary(), quiescent());
        assert!(!workspace.exists());
        let recovered: serde_json::Value =
            serde_json::from_slice(&fs::read(marker_path).unwrap()).unwrap();
        assert_eq!(recovered["state"], "recovered");
        assert!(recovered["workspace"].is_null());
    }

    #[test]
    fn native_reader_rejects_noncanonical_or_unknown_ledger_state() {
        let fixture = Fixture::new();
        let ledger = fixture._root.0.join(
            "Library/Application Support/dev.rohtolos.saturday/Stage1OwnerAuthorization/ledger",
        );
        let marker = ledger.join(format!(".saturday-stage1-{}.json", "a".repeat(32)));
        fs::write(&marker, b"{}").unwrap();
        fs::set_permissions(
            &marker,
            <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .unwrap();
        assert!(matches!(
            NativeLedgerStateReader.lock_recover_and_summarize(&fixture.directories),
            Err(OwnerAuthorizationError::Recovery)
        ));
    }

    #[test]
    fn successful_delete_is_user_present_locked_confirmed_then_removes_bootstrap() {
        let fixture = Fixture::new();
        let reader = fixture.reader(Ok(quiescent()));
        let store = fixture.store(Ok(()));
        let backend = FakeBackend::new(
            fixture.key.clone(),
            DeleteBehavior::SuccessAbsent,
            Arc::clone(&fixture.lock_active),
        );

        assert_eq!(
            delete_owner_key(
                &fixture.directories,
                &backend,
                &fixture.anchor,
                &reader,
                &store,
                [0x5a; 32],
            ),
            Ok(())
        );
        assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.verified.load(Ordering::SeqCst), 1);
        assert_eq!(store.removed.load(Ordering::SeqCst), 1);
        assert!(!fixture.lock_active.load(Ordering::SeqCst));
        assert_eq!(
            backend.calls.lock().unwrap().as_slice(),
            [BackendCall::Copy, BackendCall::Delete, BackendCall::Copy]
        );
        let challenge = backend.challenge.lock().unwrap().clone().unwrap();
        let key_id = match fixture.anchor {
            PublicKeyAnchor::Pinned { key_id, .. } => key_id,
            PublicKeyAnchor::Unprovisioned => unreachable!(),
        };
        let summary_sha256 =
            decode_hex_32("9d770f3a9cddda23eb380bd70123136ba9782eff3f67ce9ccf74cb3618c9801d")
                .unwrap();
        let mut expected_challenge = b"SATURDAY-STAGE1-OWNER-AUTHORITY-DELETE-V1\0".to_vec();
        expected_challenge.extend_from_slice(&key_id);
        expected_challenge.extend_from_slice(&summary_sha256);
        assert_eq!(challenge, expected_challenge);
        assert_eq!(
            backend.prompt.lock().unwrap().as_deref(),
            Some(DELETE_PROMPT)
        );
    }

    #[test]
    fn active_or_invalid_state_refuses_before_user_presence_or_bootstrap_removal() {
        for summary in [
            LedgerStateSummary {
                active_or_prepared_generations: 1,
                ..quiescent()
            },
            LedgerStateSummary {
                valid: false,
                ..quiescent()
            },
        ] {
            let fixture = Fixture::new();
            let reader = fixture.reader(Ok(summary));
            let store = fixture.store(Ok(()));
            let backend = FakeBackend::new(
                fixture.key.clone(),
                DeleteBehavior::SuccessAbsent,
                Arc::clone(&fixture.lock_active),
            );
            assert_eq!(
                delete_owner_key(
                    &fixture.directories,
                    &backend,
                    &fixture.anchor,
                    &reader,
                    &store,
                    [0x5a; 32],
                ),
                Err(OwnerAuthorizationError::DeleteRefused)
            );
            assert_eq!(store.removed.load(Ordering::SeqCst), 0);
            assert_eq!(
                backend.calls.lock().unwrap().as_slice(),
                [BackendCall::Copy]
            );
        }
    }

    #[test]
    fn completed_receipts_are_quiescent_and_remain_after_authenticated_delete() {
        let fixture = Fixture::new();
        let reader = fixture.reader(Ok(LedgerStateSummary {
            committed_or_published_receipts: 7,
            ..quiescent()
        }));
        let store = fixture.store(Ok(()));
        let backend = FakeBackend::new(
            fixture.key.clone(),
            DeleteBehavior::SuccessAbsent,
            Arc::clone(&fixture.lock_active),
        );
        assert_eq!(
            delete_owner_key(
                &fixture.directories,
                &backend,
                &fixture.anchor,
                &reader,
                &store,
                [0x5a; 32],
            ),
            Ok(())
        );
        assert_eq!(store.removed.load(Ordering::SeqCst), 1);
        assert_eq!(
            backend.calls.lock().unwrap().as_slice(),
            [BackendCall::Copy, BackendCall::Delete, BackendCall::Copy]
        );
    }

    #[test]
    fn recovery_failure_or_incomplete_recovery_never_reaches_delete() {
        for (result, expected) in [
            (
                Err(OwnerAuthorizationError::Recovery),
                OwnerAuthorizationError::Recovery,
            ),
            (
                Ok(LedgerStateSummary {
                    recovery_complete: false,
                    ..quiescent()
                }),
                OwnerAuthorizationError::DeleteRefused,
            ),
        ] {
            let fixture = Fixture::new();
            let reader = fixture.reader(result);
            let store = fixture.store(Ok(()));
            let backend = FakeBackend::new(
                fixture.key.clone(),
                DeleteBehavior::SuccessAbsent,
                Arc::clone(&fixture.lock_active),
            );
            assert_eq!(
                delete_owner_key(
                    &fixture.directories,
                    &backend,
                    &fixture.anchor,
                    &reader,
                    &store,
                    [0x5a; 32],
                ),
                Err(expected)
            );
            assert_eq!(store.removed.load(Ordering::SeqCst), 0);
            assert_eq!(
                backend.calls.lock().unwrap().as_slice(),
                [BackendCall::Copy]
            );
        }
    }

    #[test]
    fn cancellation_deletion_failure_and_confirmation_failure_preserve_bootstrap() {
        for (behavior, expected) in [
            (
                DeleteBehavior::UserCancelled,
                OwnerAuthorizationError::UserCancelled,
            ),
            (
                DeleteBehavior::Failure,
                OwnerAuthorizationError::Authentication,
            ),
            (
                DeleteBehavior::SuccessStillPresent,
                OwnerAuthorizationError::DeleteRefused,
            ),
        ] {
            let fixture = Fixture::new();
            let reader = fixture.reader(Ok(quiescent()));
            let store = fixture.store(Ok(()));
            let backend = FakeBackend::new(
                fixture.key.clone(),
                behavior,
                Arc::clone(&fixture.lock_active),
            );
            assert_eq!(
                delete_owner_key(
                    &fixture.directories,
                    &backend,
                    &fixture.anchor,
                    &reader,
                    &store,
                    [0x5a; 32],
                ),
                Err(expected)
            );
            assert_eq!(store.removed.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn confirmed_deletion_with_bootstrap_removal_failure_is_fixed_incomplete_state() {
        let fixture = Fixture::new();
        let reader = fixture.reader(Ok(quiescent()));
        let store = fixture.store(Err(OwnerAuthorizationError::CanonicalStorage));
        let backend = FakeBackend::new(
            fixture.key.clone(),
            DeleteBehavior::SuccessAbsent,
            Arc::clone(&fixture.lock_active),
        );
        assert_eq!(
            delete_owner_key(
                &fixture.directories,
                &backend,
                &fixture.anchor,
                &reader,
                &store,
                [0x5a; 32],
            ),
            Err(OwnerAuthorizationError::DeleteIncomplete)
        );
        assert!(backend.absent.load(Ordering::SeqCst));
        assert_eq!(store.removed.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn key_or_bootstrap_mismatch_refuses_before_lock_and_delete() {
        let fixture = Fixture::new();
        let reader = fixture.reader(Ok(quiescent()));
        let store = FakeBootstrapStore {
            verify_result: Err(OwnerAuthorizationError::KeyMismatch),
            remove_result: Ok(()),
            lock_active: Arc::clone(&fixture.lock_active),
            verified: AtomicU64::new(0),
            removed: AtomicU64::new(0),
        };
        let backend = FakeBackend::new(
            fixture.key.clone(),
            DeleteBehavior::SuccessAbsent,
            Arc::clone(&fixture.lock_active),
        );
        assert_eq!(
            delete_owner_key(
                &fixture.directories,
                &backend,
                &fixture.anchor,
                &reader,
                &store,
                [0x5a; 32],
            ),
            Err(OwnerAuthorizationError::KeyMismatch)
        );
        assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.removed.load(Ordering::SeqCst), 0);

        let mut wrong = fixture.key.clone();
        wrong[64] ^= 1;
        let matching_store = fixture.store(Ok(()));
        let mismatched_backend = FakeBackend::new(
            wrong,
            DeleteBehavior::SuccessAbsent,
            Arc::clone(&fixture.lock_active),
        );
        assert_eq!(
            delete_owner_key(
                &fixture.directories,
                &mismatched_backend,
                &fixture.anchor,
                &reader,
                &matching_store,
                [0x5a; 32],
            ),
            Err(OwnerAuthorizationError::KeyMismatch)
        );
        assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
        assert_eq!(matching_store.removed.load(Ordering::SeqCst), 0);
    }
}
