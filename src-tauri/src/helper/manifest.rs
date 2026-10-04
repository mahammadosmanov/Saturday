use crate::helper::protocol::PROTOCOL_VERSION;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

pub const MANIFEST_SCHEMA: &str = "saturday.helper-candidate.v1";
pub const EVIDENCE_SCHEMA: &str = "saturday.helper-evidence.v1";
pub const LAUNCHER_POLICY_ID: &str = "macos-deny-network-no-fork-v2";
pub const CANCEL_ACK_TIMEOUT_MS: u64 = 250;
pub const SHUTDOWN_TIMEOUT_MS: u64 = 2_000;
pub const STARTUP_TIMEOUT_MS: u64 = 2_000;
pub const PROBE_TIMEOUT_MS: u64 = 5_000;
pub const MAX_OUTPUT_BYTES: u64 = 1_048_576;
pub const MAX_CHUNKS: u32 = 256;
pub const MAX_STDERR_BYTES: u64 = 1_048_576;
pub const MAX_PROCESS_COUNT: u32 = 8;
pub const MAX_PEAK_RSS_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateManifestV1 {
    pub schema: String,
    pub protocol_version: u16,
    pub run_id: String,
    pub helper_build_id: String,
    pub executable_relative_path: String,
    pub executable_size_bytes: u64,
    pub executable_sha256: String,
    pub platform: String,
    pub architecture: String,
    pub cancel_ack_timeout_ms: u64,
    pub shutdown_timeout_ms: u64,
    pub startup_timeout_ms: u64,
    pub probe_timeout_ms: u64,
    pub max_output_bytes: u64,
    pub max_chunks: u32,
    pub max_stderr_bytes: u64,
    pub max_process_count: u32,
    pub max_peak_rss_bytes: u64,
    pub launcher_policy_id: String,
    pub evidence_schema: String,
}

#[derive(Debug)]
pub struct FrozenCandidate {
    manifest: CandidateManifestV1,
    root: PathBuf,
    executable: PathBuf,
    executable_file: File,
    root_device: u64,
    root_inode: u64,
    executable_device: u64,
    executable_inode: u64,
    manifest_sha256: String,
}

impl FrozenCandidate {
    pub fn manifest(&self) -> &CandidateManifestV1 {
        &self.manifest
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn revalidate_executable(&self) -> Result<(), ManifestError> {
        let root_metadata = fs::symlink_metadata(&self.root)?;
        if root_metadata.file_type().is_symlink()
            || !root_metadata.is_dir()
            || root_metadata.uid() != effective_uid()
            || root_metadata.mode() & 0o777 != 0o500
            || root_metadata.dev() != self.root_device
            || root_metadata.ino() != self.root_inode
        {
            return Err(ManifestError::IdentityMismatch);
        }
        let path_metadata = fs::symlink_metadata(&self.executable)?;
        if path_metadata.file_type().is_symlink()
            || !path_metadata.is_file()
            || path_metadata.dev() != self.executable_device
            || path_metadata.ino() != self.executable_inode
            || path_metadata.nlink() != 1
            || path_metadata.len() != self.manifest.executable_size_bytes
        {
            return Err(ManifestError::IdentityMismatch);
        }
        let mut held = self.executable_file.try_clone()?;
        held.rewind()?;
        if sha256_reader(&mut held)? != self.manifest.executable_sha256 {
            return Err(ManifestError::IdentityMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest path is invalid")]
    InvalidPath,
    #[error("manifest identity does not match executable")]
    IdentityMismatch,
    #[error("unsupported manifest field: {0}")]
    Unsupported(&'static str),
    #[error("invalid manifest: {0}")]
    InvalidManifest(&'static str),
    #[error("manifest I/O failure: {0}")]
    Io(#[from] io::Error),
    #[error("manifest serialization failed")]
    Serialization,
}

pub fn freeze_candidate(
    root: &Path,
    manifest: CandidateManifestV1,
) -> Result<FrozenCandidate, ManifestError> {
    validate_manifest(&manifest)?;
    let root_metadata = fs::symlink_metadata(root)?;
    if root_metadata.file_type().is_symlink()
        || !root_metadata.is_dir()
        || root_metadata.uid() != effective_uid()
        || root_metadata.mode() & 0o777 != 0o700
    {
        return Err(ManifestError::InvalidPath);
    }
    let canonical_root = root.canonicalize()?;
    let relative = validate_relative_path(&manifest.executable_relative_path)?;
    let unresolved = canonical_root.join(relative);
    let metadata = fs::symlink_metadata(&unresolved)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o111 == 0
        || metadata.nlink() != 1
    {
        return Err(ManifestError::InvalidPath);
    }
    let executable = unresolved.canonicalize()?;
    if executable != unresolved || executable.strip_prefix(&canonical_root).is_err() {
        return Err(ManifestError::InvalidPath);
    }
    let executable_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&executable)?;
    let held_metadata = executable_file.metadata()?;
    if held_metadata.dev() != metadata.dev() || held_metadata.ino() != metadata.ino() {
        return Err(ManifestError::IdentityMismatch);
    }
    if metadata.len() != manifest.executable_size_bytes
        || sha256_file(&executable)? != manifest.executable_sha256
    {
        return Err(ManifestError::IdentityMismatch);
    }
    fs::set_permissions(&canonical_root, fs::Permissions::from_mode(0o500))?;
    let sealed_root_metadata = fs::symlink_metadata(&canonical_root)?;
    let manifest_sha256 = sha256_bytes(&canonical_json(&manifest)?);
    Ok(FrozenCandidate {
        manifest,
        root: canonical_root,
        executable,
        executable_file,
        root_device: sealed_root_metadata.dev(),
        root_inode: sealed_root_metadata.ino(),
        executable_device: metadata.dev(),
        executable_inode: metadata.ino(),
        manifest_sha256,
    })
}

fn validate_manifest(manifest: &CandidateManifestV1) -> Result<(), ManifestError> {
    if manifest.schema != MANIFEST_SCHEMA {
        return Err(ManifestError::Unsupported("schema"));
    }
    if manifest.protocol_version != PROTOCOL_VERSION {
        return Err(ManifestError::Unsupported("protocol_version"));
    }
    if manifest.platform != std::env::consts::OS {
        return Err(ManifestError::Unsupported("platform"));
    }
    if manifest.architecture != std::env::consts::ARCH {
        return Err(ManifestError::Unsupported("architecture"));
    }
    if manifest.launcher_policy_id != LAUNCHER_POLICY_ID {
        return Err(ManifestError::Unsupported("launcher_policy_id"));
    }
    if manifest.evidence_schema != EVIDENCE_SCHEMA {
        return Err(ManifestError::Unsupported("evidence_schema"));
    }
    if manifest.cancel_ack_timeout_ms != CANCEL_ACK_TIMEOUT_MS
        || manifest.shutdown_timeout_ms != SHUTDOWN_TIMEOUT_MS
        || manifest.startup_timeout_ms != STARTUP_TIMEOUT_MS
        || manifest.probe_timeout_ms != PROBE_TIMEOUT_MS
        || manifest.max_output_bytes != MAX_OUTPUT_BYTES
        || manifest.max_chunks != MAX_CHUNKS
        || manifest.max_stderr_bytes != MAX_STDERR_BYTES
        || manifest.max_process_count != MAX_PROCESS_COUNT
        || manifest.max_peak_rss_bytes != MAX_PEAK_RSS_BYTES
    {
        return Err(ManifestError::InvalidManifest("timeout budgets"));
    }
    if manifest.executable_size_bytes == 0 {
        return Err(ManifestError::InvalidManifest("empty executable"));
    }
    if !is_run_id(&manifest.run_id) || !is_identifier(&manifest.helper_build_id) {
        return Err(ManifestError::InvalidManifest("identifier"));
    }
    if !is_sha256(&manifest.executable_sha256) {
        return Err(ManifestError::InvalidManifest("executable SHA-256"));
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<PathBuf, ManifestError> {
    let path = Path::new(value);
    if value.is_empty() || path.is_absolute() || value.len() > 255 {
        return Err(ManifestError::InvalidPath);
    }
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(path.to_path_buf()),
        _ => Err(ManifestError::InvalidPath),
    }
}

pub fn generate_run_id() -> Result<String, ManifestError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| ManifestError::InvalidManifest("OS randomness"))?;
    Ok(format_digest(bytes))
}

fn is_run_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ManifestError> {
    let value = serde_json::to_value(value).map_err(|_| ManifestError::Serialization)?;
    serde_json::to_vec(&value).map_err(|_| ManifestError::Serialization)
}

pub fn sha256_file(path: &Path) -> Result<String, io::Error> {
    let mut file = File::open(path)?;
    sha256_reader(&mut file)
}

fn sha256_reader(reader: &mut impl Read) -> Result<String, io::Error> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format_digest(hasher.finalize()))
}

fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no arguments and no memory-safety preconditions.
    unsafe { libc::geteuid() }
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    format_digest(Sha256::digest(bytes))
}

fn format_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::PathBuf;

    fn private_root(label: &str) -> PathBuf {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).unwrap();
        let suffix = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let root = std::env::temp_dir().join(format!("saturday-{label}-{suffix}"));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    fn candidate(root: &std::path::Path) -> CandidateManifestV1 {
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        let executable = root.join("fixture-helper");
        if executable.exists() {
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(&executable, b"fixture-bytes").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o500)).unwrap();
        CandidateManifestV1 {
            schema: MANIFEST_SCHEMA.to_owned(),
            protocol_version: crate::helper::protocol::PROTOCOL_VERSION,
            run_id: generate_run_id().unwrap(),
            helper_build_id: "fixture-1".to_owned(),
            executable_relative_path: "fixture-helper".to_owned(),
            executable_size_bytes: 13,
            executable_sha256: sha256_file(&executable).unwrap(),
            platform: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            cancel_ack_timeout_ms: 250,
            shutdown_timeout_ms: 2_000,
            startup_timeout_ms: STARTUP_TIMEOUT_MS,
            probe_timeout_ms: PROBE_TIMEOUT_MS,
            max_output_bytes: MAX_OUTPUT_BYTES,
            max_chunks: MAX_CHUNKS,
            max_stderr_bytes: MAX_STDERR_BYTES,
            max_process_count: MAX_PROCESS_COUNT,
            max_peak_rss_bytes: MAX_PEAK_RSS_BYTES,
            launcher_policy_id: LAUNCHER_POLICY_ID.to_owned(),
            evidence_schema: "saturday.helper-evidence.v1".to_owned(),
        }
    }

    fn remove_private(root: &std::path::Path) {
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn freezes_exact_regular_executable_identity() {
        let root = private_root("manifest-valid");
        let manifest = candidate(&root);
        let frozen = freeze_candidate(&root, manifest.clone()).unwrap();
        assert_eq!(frozen.manifest(), &manifest);
        assert_eq!(
            frozen.executable(),
            root.join("fixture-helper").canonicalize().unwrap()
        );
        assert_eq!(frozen.manifest_sha256().len(), 64);
        remove_private(&root);
    }

    #[test]
    fn rejects_traversal_symlink_and_hash_mismatch() {
        let root = private_root("manifest-invalid");
        let mut traversal = candidate(&root);
        traversal.executable_relative_path = "../fixture-helper".to_owned();
        assert!(matches!(
            freeze_candidate(&root, traversal),
            Err(ManifestError::InvalidPath)
        ));

        let target = root.join("target");
        fs::write(&target, b"fixture-bytes").unwrap();
        let link = root.join("link");
        symlink(&target, &link).unwrap();
        let mut linked = candidate(&root);
        linked.executable_relative_path = "link".to_owned();
        assert!(matches!(
            freeze_candidate(&root, linked),
            Err(ManifestError::InvalidPath)
        ));

        let mut mismatch = candidate(&root);
        mismatch.executable_sha256 = "0".repeat(64);
        assert!(matches!(
            freeze_candidate(&root, mismatch),
            Err(ManifestError::IdentityMismatch)
        ));
        remove_private(&root);
    }

    #[test]
    fn rejects_unsupported_identity_and_budgets() {
        let root = private_root("manifest-policy");
        let mut manifest = candidate(&root);
        manifest.protocol_version += 1;
        assert!(matches!(
            freeze_candidate(&root, manifest),
            Err(ManifestError::Unsupported(_))
        ));

        let mut manifest = candidate(&root);
        manifest.cancel_ack_timeout_ms = 0;
        assert!(matches!(
            freeze_candidate(&root, manifest),
            Err(ManifestError::InvalidManifest(_))
        ));
        remove_private(&root);
    }

    #[test]
    fn rejects_mutable_aliased_and_replaced_executables() {
        let root = private_root("manifest-mutation");
        let manifest = candidate(&root);
        let frozen = freeze_candidate(&root, manifest).unwrap();
        fs::set_permissions(frozen.executable(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(frozen.executable(), b"changed-bytes").unwrap();
        fs::set_permissions(frozen.executable(), fs::Permissions::from_mode(0o500)).unwrap();
        assert!(matches!(
            frozen.revalidate_executable(),
            Err(ManifestError::IdentityMismatch)
        ));
        drop(frozen);

        let manifest = candidate(&root);
        let external = private_root("manifest-hardlink");
        fs::hard_link(root.join("fixture-helper"), external.join("alias")).unwrap();
        assert!(matches!(
            freeze_candidate(&root, manifest),
            Err(ManifestError::InvalidPath)
        ));
        fs::remove_dir_all(external).unwrap();

        let manifest = candidate(&root);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(matches!(
            freeze_candidate(&root, manifest),
            Err(ManifestError::InvalidPath)
        ));
        remove_private(&root);
    }
}
