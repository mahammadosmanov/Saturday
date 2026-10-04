use crate::owner_authorization::canonical::decode_lower_hex;
use std::ffi::{CStr, CString, OsString};
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr;
use thiserror::Error;

const APPLICATION_COMPONENTS: [&str; 2] = ["Library", "Application Support"];
const APP_SUPPORT_ROOT: &str = "dev.rohtolos.jarvis";
const AUTHORIZATION_ROOT: &str = "Stage1OwnerAuthorization";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryIdentity {
    pub device: u64,
    pub inode: u64,
    pub uid: u32,
}

#[derive(Debug)]
pub struct Stage1Directories {
    pub root: OwnedFd,
    pub bootstrap: OwnedFd,
    pub ledger: OwnedFd,
    pub workspace_parent: OwnedFd,
    pub receipt_parent: OwnedFd,
    pub ledger_identity: DirectoryIdentity,
    pub workspace_parent_identity: DirectoryIdentity,
    pub receipt_parent_identity: DirectoryIdentity,
}

pub trait AccountHomeResolver {
    fn effective_user_home(&self) -> Result<PathBuf, PathError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemAccountHomeResolver;

impl AccountHomeResolver for SystemAccountHomeResolver {
    fn effective_user_home(&self) -> Result<PathBuf, PathError> {
        let uid = effective_uid();
        let requested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
        let buffer_size = if requested > 0 {
            usize::try_from(requested).map_err(|_| PathError::AccountRecord)?
        } else {
            16 * 1024
        };
        if buffer_size == 0 || buffer_size > 1024 * 1024 {
            return Err(PathError::AccountRecord);
        }

        let mut buffer = vec![0_u8; buffer_size];
        let mut record = MaybeUninit::<libc::passwd>::uninit();
        let mut result = ptr::null_mut();
        // SAFETY: each pointer references writable storage for the duration of the call;
        // getpwuid_r initializes record and result on success.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                record.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status != 0 || result.is_null() {
            return Err(PathError::AccountRecord);
        }
        // SAFETY: a non-null result from successful getpwuid_r points at initialized record data.
        let record = unsafe { record.assume_init() };
        if record.pw_dir.is_null() {
            return Err(PathError::AccountRecord);
        }
        // SAFETY: pw_dir is a NUL-terminated string backed by buffer until this function returns.
        let bytes = unsafe { CStr::from_ptr(record.pw_dir) }.to_bytes();
        if bytes.is_empty() {
            return Err(PathError::AccountRecord);
        }
        let home = PathBuf::from(OsString::from_vec(bytes.to_vec()));
        if !home.is_absolute() {
            return Err(PathError::AccountRecord);
        }
        Ok(home)
    }
}

#[derive(Debug, Error)]
pub enum PathError {
    #[error("effective account record is unavailable")]
    AccountRecord,
    #[error("canonical account home is invalid")]
    InvalidHome,
    #[error("canonical directory operation failed")]
    Io(#[source] io::Error),
    #[error("canonical directory is unsafe")]
    UnsafeDirectory,
    #[error("canonical directory roles alias")]
    AliasedDirectory,
    #[error("generation is invalid")]
    InvalidGeneration,
}

pub fn open_stage1_directories(
    resolver: &impl AccountHomeResolver,
) -> Result<Stage1Directories, PathError> {
    let uid = effective_uid();
    let home = resolver.effective_user_home()?;
    if !home.is_absolute() {
        return Err(PathError::InvalidHome);
    }
    let home_name =
        CString::new(home.as_os_str().as_bytes()).map_err(|_| PathError::InvalidHome)?;
    // SAFETY: home_name is NUL-terminated; successful open returns a newly owned descriptor.
    let home_fd = unsafe {
        libc::open(
            home_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if home_fd < 0 {
        return Err(PathError::Io(io::Error::last_os_error()));
    }
    // SAFETY: home_fd was returned by open and ownership transfers to OwnedFd.
    let mut current = unsafe { OwnedFd::from_raw_fd(home_fd) };
    validate_account_directory(current.as_fd(), uid)?;

    for component in APPLICATION_COMPONENTS {
        current = open_or_create_directory(current.as_fd(), component, uid, false)?;
    }
    current = open_or_create_directory(current.as_fd(), APP_SUPPORT_ROOT, uid, false)?;
    current = open_or_create_directory(current.as_fd(), AUTHORIZATION_ROOT, uid, true)?;

    let root_identity = validate_private_directory(current.as_fd(), uid)?;
    let (bootstrap, bootstrap_identity) = open_private_child(current.as_fd(), "bootstrap", uid)?;
    let (ledger, ledger_identity) = open_private_child(current.as_fd(), "ledger", uid)?;
    let (workspace_parent, workspace_parent_identity) =
        open_private_child(current.as_fd(), "workspace", uid)?;
    let (receipt_parent, receipt_parent_identity) =
        open_private_child(current.as_fd(), "receipts", uid)?;

    ensure_distinct(&[
        root_identity,
        bootstrap_identity,
        ledger_identity,
        workspace_parent_identity,
        receipt_parent_identity,
    ])?;

    Ok(Stage1Directories {
        root: current,
        bootstrap,
        ledger,
        workspace_parent,
        receipt_parent,
        ledger_identity,
        workspace_parent_identity,
        receipt_parent_identity,
    })
}

pub fn generation_marker_name(generation: &str) -> Result<String, PathError> {
    validate_generation(generation)?;
    Ok(format!(".jarvis-stage1-generation-{generation}.json"))
}

pub fn workspace_name(generation: &str) -> Result<String, PathError> {
    validate_generation(generation)?;
    Ok(format!("jarvis-prebuilt-{generation}"))
}

pub fn receipt_name(generation: &str) -> Result<String, PathError> {
    validate_generation(generation)?;
    Ok(format!("stage1-{generation}.json"))
}

fn open_private_child(
    parent: BorrowedFd<'_>,
    name: &str,
    uid: u32,
) -> Result<(OwnedFd, DirectoryIdentity), PathError> {
    let descriptor = open_or_create_directory(parent, name, uid, true)?;
    let identity = validate_private_directory(descriptor.as_fd(), uid)?;
    Ok((descriptor, identity))
}

fn open_or_create_directory(
    parent: BorrowedFd<'_>,
    name: &str,
    uid: u32,
    private: bool,
) -> Result<OwnedFd, PathError> {
    let name = CString::new(name).map_err(|_| PathError::UnsafeDirectory)?;
    let mut created = false;
    let descriptor = match open_directory_at(parent, &name) {
        Ok(descriptor) => descriptor,
        Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {
            // SAFETY: parent is open and name is a single NUL-terminated component.
            let status = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
            if status == 0 {
                created = true;
            } else {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::EEXIST) {
                    return Err(PathError::Io(error));
                }
            }
            open_directory_at(parent, &name).map_err(PathError::Io)?
        }
        Err(error) => return Err(PathError::Io(error)),
    };

    if created {
        // SAFETY: descriptor is an open directory owned by this process.
        if unsafe { libc::fchmod(descriptor.as_raw_fd(), 0o700) } != 0 {
            return Err(PathError::Io(io::Error::last_os_error()));
        }
    }
    if private {
        validate_private_directory(descriptor.as_fd(), uid)?;
    } else {
        validate_account_directory(descriptor.as_fd(), uid)?;
    }
    Ok(descriptor)
}

fn open_directory_at(parent: BorrowedFd<'_>, name: &CStr) -> Result<OwnedFd, io::Error> {
    // SAFETY: parent is open and name is NUL-terminated; successful openat returns a new fd.
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: descriptor was returned by openat and ownership transfers to OwnedFd.
        Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
    }
}

fn validate_account_directory(
    descriptor: BorrowedFd<'_>,
    expected_uid: u32,
) -> Result<DirectoryIdentity, PathError> {
    let (identity, mode) = directory_identity(descriptor)?;
    if identity.uid != expected_uid || mode & 0o022 != 0 {
        return Err(PathError::UnsafeDirectory);
    }
    Ok(identity)
}

fn validate_private_directory(
    descriptor: BorrowedFd<'_>,
    expected_uid: u32,
) -> Result<DirectoryIdentity, PathError> {
    let (identity, mode) = directory_identity(descriptor)?;
    if identity.uid != expected_uid || mode & 0o777 != 0o700 {
        return Err(PathError::UnsafeDirectory);
    }
    Ok(identity)
}

fn directory_identity(
    descriptor: BorrowedFd<'_>,
) -> Result<(DirectoryIdentity, libc::mode_t), PathError> {
    let mut metadata = MaybeUninit::<libc::stat>::uninit();
    // SAFETY: metadata points to writable stat storage and descriptor is open.
    if unsafe { libc::fstat(descriptor.as_raw_fd(), metadata.as_mut_ptr()) } != 0 {
        return Err(PathError::Io(io::Error::last_os_error()));
    }
    // SAFETY: fstat succeeded and initialized metadata.
    let metadata = unsafe { metadata.assume_init() };
    if metadata.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return Err(PathError::UnsafeDirectory);
    }
    Ok((
        DirectoryIdentity {
            device: metadata.st_dev as u64,
            inode: metadata.st_ino,
            uid: metadata.st_uid,
        },
        metadata.st_mode,
    ))
}

fn ensure_distinct(identities: &[DirectoryIdentity]) -> Result<(), PathError> {
    for (index, identity) in identities.iter().enumerate() {
        if identities[..index].contains(identity) {
            return Err(PathError::AliasedDirectory);
        }
    }
    Ok(())
}

fn validate_generation(generation: &str) -> Result<(), PathError> {
    decode_lower_hex::<16>(generation)
        .map(|_| ())
        .map_err(|_| PathError::InvalidGeneration)
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no arguments and no memory-safety preconditions.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::fd::AsFd;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);
    static ENVIRONMENT_LOCK: Mutex<()> = Mutex::new(());

    struct FakeResolver(PathBuf);

    impl AccountHomeResolver for FakeResolver {
        fn effective_user_home(&self) -> Result<PathBuf, PathError> {
            Ok(self.0.clone())
        }
    }

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "jarvis-owner-paths-{label}-{}-{}",
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

    #[test]
    fn ignores_hostile_home_environment_and_uses_account_record() {
        let _environment = ENVIRONMENT_LOCK.lock().unwrap();
        let account_home = TestRoot::new("account-home");
        let hostile_home = TestRoot::new("hostile-home");
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", hostile_home.path());

        let directories = open_stage1_directories(&FakeResolver(account_home.path().into()))
            .expect("fake account home must be used");
        drop(directories);

        if let Some(previous) = previous {
            std::env::set_var("HOME", previous);
        } else {
            std::env::remove_var("HOME");
        }

        assert!(account_home.path().join(relative_root()).is_dir());
        assert!(!hostile_home.path().join(relative_root()).exists());
    }

    #[test]
    fn creates_exact_private_children_and_returns_distinct_identities() {
        let home = TestRoot::new("exact-tree");
        let directories =
            open_stage1_directories(&FakeResolver(home.path().into())).expect("tree must open");
        let root = home.path().join(relative_root());

        let mut names = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, ["bootstrap", "ledger", "receipts", "workspace"]);
        for name in &names {
            let metadata = fs::symlink_metadata(root.join(name)).unwrap();
            assert!(metadata.is_dir());
            assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
        }

        assert_ne!(
            directories.ledger_identity,
            directories.workspace_parent_identity
        );
        assert_ne!(
            directories.ledger_identity,
            directories.receipt_parent_identity
        );
        assert_ne!(
            directories.workspace_parent_identity,
            directories.receipt_parent_identity
        );
    }

    #[test]
    fn accepts_existing_owner_only_app_support_root_and_keeps_stage1_private() {
        let home = TestRoot::new("existing-app-root");
        let app_root = home
            .path()
            .join("Library/Application Support/dev.rohtolos.jarvis");
        fs::create_dir_all(&app_root).unwrap();
        fs::set_permissions(&app_root, fs::Permissions::from_mode(0o755)).unwrap();

        let directories =
            open_stage1_directories(&FakeResolver(home.path().into())).expect("tree must open");
        drop(directories);

        assert_eq!(
            fs::symlink_metadata(&app_root)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            fs::symlink_metadata(home.path().join(relative_root()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn rejects_symlink_wrong_owner_wrong_mode_regular_file_and_alias_roles() {
        let symlink_home = TestRoot::new("symlink");
        let outside = TestRoot::new("outside");
        fs::create_dir_all(symlink_home.path().join("Library/Application Support")).unwrap();
        symlink(
            outside.path(),
            symlink_home
                .path()
                .join("Library/Application Support/dev.rohtolos.jarvis"),
        )
        .unwrap();
        assert!(open_stage1_directories(&FakeResolver(symlink_home.path().into())).is_err());

        let file_home = TestRoot::new("file");
        fs::create_dir_all(file_home.path().join("Library/Application Support")).unwrap();
        fs::write(
            file_home
                .path()
                .join("Library/Application Support/dev.rohtolos.jarvis"),
            b"not a directory",
        )
        .unwrap();
        assert!(open_stage1_directories(&FakeResolver(file_home.path().into())).is_err());

        let mode_home = TestRoot::new("wrong-mode");
        drop(open_stage1_directories(&FakeResolver(mode_home.path().into())).unwrap());
        let ledger = mode_home.path().join(relative_root()).join("ledger");
        fs::set_permissions(&ledger, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(open_stage1_directories(&FakeResolver(mode_home.path().into())).is_err());

        let alias_home = TestRoot::new("alias");
        drop(open_stage1_directories(&FakeResolver(alias_home.path().into())).unwrap());
        let root = alias_home.path().join(relative_root());
        fs::remove_dir(root.join("receipts")).unwrap();
        symlink("workspace", root.join("receipts")).unwrap();
        assert!(open_stage1_directories(&FakeResolver(alias_home.path().into())).is_err());

        let owner_home = TestRoot::new("wrong-owner");
        let directories = open_stage1_directories(&FakeResolver(owner_home.path().into())).unwrap();
        let wrong_uid = directories.ledger_identity.uid.wrapping_add(1);
        assert!(validate_private_directory(directories.ledger.as_fd(), wrong_uid).is_err());
    }

    #[test]
    fn fixed_names_accept_only_a_generation_hex_value() {
        let generation = "00112233445566778899aabbccddeeff";
        assert_eq!(
            generation_marker_name(generation).unwrap(),
            ".jarvis-stage1-generation-00112233445566778899aabbccddeeff.json"
        );
        assert_eq!(
            workspace_name(generation).unwrap(),
            "jarvis-prebuilt-00112233445566778899aabbccddeeff"
        );
        assert_eq!(
            receipt_name(generation).unwrap(),
            "stage1-00112233445566778899aabbccddeeff.json"
        );
        for invalid in [
            "00112233445566778899AABBCCDDEEFF",
            "00112233445566778899aabbccddeef",
            "00112233445566778899aabbccddeeff00",
            "../../00112233445566778899aabbccdd",
        ] {
            assert!(generation_marker_name(invalid).is_err());
            assert!(workspace_name(invalid).is_err());
            assert!(receipt_name(invalid).is_err());
        }
    }

    fn relative_root() -> PathBuf {
        PathBuf::from("Library/Application Support/dev.rohtolos.jarvis/Stage1OwnerAuthorization")
    }
}
