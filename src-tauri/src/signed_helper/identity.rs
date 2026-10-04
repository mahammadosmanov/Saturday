use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use thiserror::Error;

const IDENTITY_SCHEMA: &str = "jarvis.helper-identity.v1";
const HELPER_NAME: &str = "jarvis-runtime-helper";
const TARGET_TRIPLE: &str = "aarch64-apple-darwin";
const MAX_IDENTITY_BYTES: usize = 4096;
const MAX_HELPER_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperIdentity {
    schema: String,
    helper_name: String,
    protocol_version: u16,
    target_triple: String,
    pub source_generation_sha256: String,
    pub binary_sha256: String,
    pub binary_size: u64,
}

#[derive(Debug, Error)]
pub enum HelperIdentityError {
    #[error("identity I/O failed")]
    Io(#[from] io::Error),
    #[error("identity document is invalid")]
    InvalidDocument,
    #[error("helper file identity is invalid")]
    InvalidHelper,
}

pub fn parse_identity(bytes: &[u8]) -> Result<HelperIdentity, HelperIdentityError> {
    if bytes.is_empty() || bytes.len() > MAX_IDENTITY_BYTES {
        return Err(HelperIdentityError::InvalidDocument);
    }
    let identity: HelperIdentity =
        serde_json::from_slice(bytes).map_err(|_| HelperIdentityError::InvalidDocument)?;
    if identity.schema != IDENTITY_SCHEMA
        || identity.helper_name != HELPER_NAME
        || identity.protocol_version != crate::helper::protocol::PROTOCOL_VERSION
        || identity.target_triple != TARGET_TRIPLE
        || !is_sha256(&identity.source_generation_sha256)
        || !is_sha256(&identity.binary_sha256)
        || identity.binary_size == 0
        || identity.binary_size > MAX_HELPER_BYTES
    {
        return Err(HelperIdentityError::InvalidDocument);
    }
    Ok(identity)
}

pub fn read_identity(path: &Path) -> Result<HelperIdentity, HelperIdentityError> {
    let mut file = open_no_follow(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_IDENTITY_BYTES as u64 {
        return Err(HelperIdentityError::InvalidDocument);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)?;
    parse_identity(&bytes)
}

pub fn verify_helper_file(
    path: &Path,
    identity: &HelperIdentity,
) -> Result<File, HelperIdentityError> {
    let mut file = open_no_follow(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != effective_uid()
        || metadata.nlink() != 1
        || metadata.mode() & 0o222 != 0
        || metadata.mode() & 0o111 == 0
        || metadata.len() != identity.binary_size
    {
        return Err(HelperIdentityError::InvalidHelper);
    }
    let mut digest = Sha256::new();
    let copied = io::copy(&mut file, &mut digest)?;
    if copied != identity.binary_size
        || format!("{:x}", digest.finalize()) != identity.binary_sha256
    {
        return Err(HelperIdentityError::InvalidHelper);
    }
    Ok(file)
}

fn open_no_follow(path: &Path) -> Result<File, HelperIdentityError> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(HelperIdentityError::Io)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no arguments and no memory-safety preconditions.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn valid_json(hash: &str, size: u64) -> Vec<u8> {
        format!(
            "{{\"binary_sha256\":\"{hash}\",\"binary_size\":{size},\"helper_name\":\"jarvis-runtime-helper\",\"protocol_version\":1,\"schema\":\"jarvis.helper-identity.v1\",\"source_generation_sha256\":\"{}\",\"target_triple\":\"aarch64-apple-darwin\"}}\n",
            "a".repeat(64)
        )
        .into_bytes()
    }

    #[test]
    fn identity_json_is_exact_and_bounded() {
        let document = parse_identity(&valid_json(&"b".repeat(64), 10)).unwrap();
        assert_eq!(document.binary_size, 10);
        let mut extra = valid_json(&"b".repeat(64), 10);
        extra.pop();
        extra.pop();
        extra.extend_from_slice(b",\"extra\":true}\n");
        assert!(parse_identity(&extra).is_err());
        assert!(parse_identity(&vec![b' '; 4097]).is_err());
    }

    #[test]
    fn helper_file_must_match_hash_size_mode_and_link_identity() {
        let root = std::env::temp_dir().join(format!(
            "jarvis-signed-helper-identity-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let helper = root.join("jarvis-runtime-helper");
        fs::write(&helper, b"helper").unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o500)).unwrap();
        let hash = format!("{:x}", sha2::Sha256::digest(b"helper"));
        let document = parse_identity(&valid_json(&hash, 6)).unwrap();
        verify_helper_file(&helper, &document).unwrap();

        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(verify_helper_file(&helper, &document).is_err());
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o500)).unwrap();
        let alias = root.join("alias");
        symlink(&helper, &alias).unwrap();
        assert!(verify_helper_file(&alias, &document).is_err());
        fs::remove_file(alias).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
