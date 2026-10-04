use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;

const PYTHON: &str = "/usr/bin/python3";
const MAX_ENVELOPE_BYTES: usize = 64 * 1024;
const MAX_STDOUT_BYTES: usize = 128;
const MAX_STDERR_BYTES: usize = 4096;
const CHILD_DEADLINE: Duration = Duration::from_secs(2);
const CLEANUP_DEADLINE: Duration = Duration::from_secs(2);
const PROCESS_POLL: Duration = Duration::from_millis(10);
const TARGET_FDS: [RawFd; 5] = [3, 4, 5, 6, 7];

pub struct Stage1ChildRequest<'a> {
    pub script: &'a Path,
    pub envelope: &'a [u8],
    pub nonce: &'a [u8; 32],
    pub ledger_fd: BorrowedFd<'a>,
    pub workspace_parent_fd: BorrowedFd<'a>,
    pub receipt_parent_fd: BorrowedFd<'a>,
}

pub trait Stage1ProcessRunner {
    fn run(&self, request: Stage1ChildRequest<'_>) -> Result<String, Stage1ProcessError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeStage1ProcessRunner;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum Stage1ProcessError {
    #[error("stage 1 child request is invalid")]
    InvalidRequest,
    #[error("stage 1 child launch failed")]
    Launch,
    #[error("stage 1 child timed out")]
    Timeout,
    #[error("stage 1 child failed")]
    ChildFailed,
    #[error("stage 1 child output is invalid")]
    InvalidOutput,
    #[error("stage 1 child left a surviving process")]
    SurvivingProcess,
    #[error("stage 1 child cleanup failed")]
    Cleanup,
}

impl Stage1ProcessRunner for NativeStage1ProcessRunner {
    fn run(&self, request: Stage1ChildRequest<'_>) -> Result<String, Stage1ProcessError> {
        validate_request(&request)?;
        let envelope_pipe = create_pipe()?;
        let nonce_pipe = create_pipe()?;
        let sources = [
            duplicate_fd(envelope_pipe.read.as_raw_fd())?,
            duplicate_fd(nonce_pipe.read.as_raw_fd())?,
            duplicate_fd(request.ledger_fd.as_raw_fd())?,
            duplicate_fd(request.workspace_parent_fd.as_raw_fd())?,
            duplicate_fd(request.receipt_parent_fd.as_raw_fd())?,
        ];
        let source_fds: [RawFd; 5] = std::array::from_fn(|index| sources[index].as_raw_fd());
        let child_write_fds = [
            envelope_pipe.write.as_raw_fd(),
            nonce_pipe.write.as_raw_fd(),
        ];

        let mut command = Command::new(PYTHON);
        command
            .arg("-B")
            .arg("-E")
            .arg("-s")
            .arg(request.script)
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear()
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .env("PATH", "/usr/bin:/bin");
        // SAFETY: the child-only closure calls async-signal-safe descriptor/process APIs with
        // fixed integer inputs and returns the immediate OS error without allocation.
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) != 0 {
                    return Err(io::Error::last_os_error());
                }
                libc::umask(0o077);
                for (source, target) in source_fds.into_iter().zip(TARGET_FDS) {
                    if libc::dup2(source, target) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    if libc::fcntl(target, libc::F_SETFD, 0) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                for descriptor in source_fds {
                    libc::close(descriptor);
                }
                for descriptor in child_write_fds {
                    if !TARGET_FDS.contains(&descriptor) {
                        libc::close(descriptor);
                    }
                }
                Ok(())
            });
        }

        let mut child = command.spawn().map_err(|_| Stage1ProcessError::Launch)?;
        let process_group = i32::try_from(child.id()).map_err(|_| Stage1ProcessError::Launch)?;
        drop(sources);
        drop(envelope_pipe.read);
        drop(nonce_pipe.read);

        let stdout = child.stdout.take().ok_or(Stage1ProcessError::Launch)?;
        let stderr = child.stderr.take().ok_or(Stage1ProcessError::Launch)?;
        let stdout_worker = spawn_bounded_reader(stdout, MAX_STDOUT_BYTES);
        let stderr_worker = spawn_bounded_reader(stderr, MAX_STDERR_BYTES);
        let envelope_worker = spawn_writer(envelope_pipe.write, request.envelope.to_vec());
        let nonce_worker = spawn_writer(nonce_pipe.write, request.nonce.to_vec());
        let mut owned = OwnedProcessGroup::new(child, process_group);

        let wait_result = wait_for_exit(&mut owned.child, CHILD_DEADLINE);
        let mut terminal_error = None;
        let status = match wait_result {
            Ok(WaitResult::Exited(status)) => {
                if wait_for_process_group_absence(process_group, Duration::from_millis(50)).is_err()
                {
                    terminal_error = Some(Stage1ProcessError::SurvivingProcess);
                    if owned.terminate().is_err() {
                        terminal_error = Some(Stage1ProcessError::Cleanup);
                    } else {
                        owned.disarm();
                    }
                }
                Some(status)
            }
            Ok(WaitResult::TimedOut) => {
                terminal_error = Some(Stage1ProcessError::Timeout);
                if owned.terminate().is_err() {
                    terminal_error = Some(Stage1ProcessError::Cleanup);
                } else {
                    owned.disarm();
                }
                None
            }
            Err(()) => {
                terminal_error = Some(Stage1ProcessError::Cleanup);
                if owned.terminate().is_ok() {
                    owned.disarm();
                }
                None
            }
        };

        let envelope_result = join_writer(envelope_worker);
        let nonce_result = join_writer(nonce_worker);
        let stdout_result = join_reader(stdout_worker);
        let stderr_result = join_reader(stderr_worker);

        if let Some(error) = terminal_error {
            return Err(error);
        }
        let status = status.ok_or(Stage1ProcessError::Cleanup)?;
        if envelope_result.is_err() || nonce_result.is_err() {
            return Err(Stage1ProcessError::ChildFailed);
        }
        let stdout = stdout_result?;
        let stderr = stderr_result?;
        if stdout.limit_exceeded || stderr.limit_exceeded {
            return Err(Stage1ProcessError::InvalidOutput);
        }
        if !status.success() {
            return Err(Stage1ProcessError::ChildFailed);
        }
        if !stderr.bytes.is_empty() {
            return Err(Stage1ProcessError::InvalidOutput);
        }
        owned.disarm();
        parse_digest(&stdout.bytes)
    }
}

fn validate_request(request: &Stage1ChildRequest<'_>) -> Result<(), Stage1ProcessError> {
    if request.envelope.is_empty() || request.envelope.len() > MAX_ENVELOPE_BYTES {
        return Err(Stage1ProcessError::InvalidRequest);
    }
    if !request.script.is_absolute()
        || request.script.file_name().and_then(|value| value.to_str())
            != Some("prebuilt_runtime_stage1.py")
    {
        return Err(Stage1ProcessError::InvalidRequest);
    }
    let metadata =
        fs::symlink_metadata(request.script).map_err(|_| Stage1ProcessError::InvalidRequest)?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(Stage1ProcessError::InvalidRequest);
    }
    Ok(())
}

struct Pipe {
    read: OwnedFd,
    write: OwnedFd,
}

fn create_pipe() -> Result<Pipe, Stage1ProcessError> {
    let mut descriptors = [-1, -1];
    // SAFETY: descriptors references writable storage for exactly two file descriptors.
    if unsafe { libc::pipe(descriptors.as_mut_ptr()) } != 0 {
        return Err(Stage1ProcessError::Launch);
    }
    for descriptor in descriptors {
        // macOS has no pipe2; set close-on-exec on both ends before either is exposed.
        if unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            // SAFETY: pipe returned both descriptors and neither has transferred ownership.
            unsafe {
                libc::close(descriptors[0]);
                libc::close(descriptors[1]);
            }
            return Err(Stage1ProcessError::Launch);
        }
    }
    // SAFETY: pipe2 succeeded and returned two newly owned descriptors.
    Ok(Pipe {
        read: unsafe { OwnedFd::from_raw_fd(descriptors[0]) },
        write: unsafe { OwnedFd::from_raw_fd(descriptors[1]) },
    })
}

fn duplicate_fd(descriptor: RawFd) -> Result<OwnedFd, Stage1ProcessError> {
    // Keep every source above the fixed target range so dup2 order cannot clobber a later source.
    let duplicated = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 8) };
    if duplicated < 0 {
        return Err(Stage1ProcessError::Launch);
    }
    // SAFETY: fcntl returned a newly owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(duplicated) })
}

fn spawn_writer(descriptor: OwnedFd, bytes: Vec<u8>) -> JoinHandle<io::Result<()>> {
    thread::spawn(move || {
        let mut file = fs::File::from(descriptor);
        file.write_all(&bytes)?;
        file.flush()
    })
}

struct BoundedOutput {
    bytes: Vec<u8>,
    limit_exceeded: bool,
}

fn spawn_bounded_reader(
    reader: impl Read + Send + 'static,
    maximum: usize,
) -> JoinHandle<io::Result<BoundedOutput>> {
    thread::spawn(move || {
        let limit = u64::try_from(maximum).unwrap_or(u64::MAX).saturating_add(1);
        let mut bytes = Vec::new();
        reader.take(limit).read_to_end(&mut bytes)?;
        let limit_exceeded = bytes.len() > maximum;
        Ok(BoundedOutput {
            bytes,
            limit_exceeded,
        })
    })
}

fn join_writer(worker: JoinHandle<io::Result<()>>) -> Result<(), Stage1ProcessError> {
    worker
        .join()
        .map_err(|_| Stage1ProcessError::Cleanup)?
        .map_err(|_| Stage1ProcessError::ChildFailed)
}

fn join_reader(
    worker: JoinHandle<io::Result<BoundedOutput>>,
) -> Result<BoundedOutput, Stage1ProcessError> {
    worker
        .join()
        .map_err(|_| Stage1ProcessError::Cleanup)?
        .map_err(|_| Stage1ProcessError::Cleanup)
}

enum WaitResult {
    Exited(ExitStatus),
    TimedOut,
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Result<WaitResult, ()> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(WaitResult::Exited(status)),
            Ok(None) if Instant::now() < deadline => thread::sleep(PROCESS_POLL),
            Ok(None) => return Ok(WaitResult::TimedOut),
            Err(_) => return Err(()),
        }
    }
}

struct OwnedProcessGroup {
    child: Child,
    process_group: i32,
    armed: bool,
}

impl OwnedProcessGroup {
    fn new(child: Child, process_group: i32) -> Self {
        Self {
            child,
            process_group,
            armed: true,
        }
    }

    fn terminate(&mut self) -> Result<(), Stage1ProcessError> {
        // SAFETY: the negative identifier targets only the process group created for this child.
        unsafe {
            libc::kill(-self.process_group, libc::SIGKILL);
        }
        let _ = self.child.wait();
        wait_for_process_group_absence(self.process_group, CLEANUP_DEADLINE)
            .map_err(|_| Stage1ProcessError::Cleanup)
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for OwnedProcessGroup {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.terminate();
        }
    }
}

fn wait_for_process_group_absence(process_group: i32, timeout: Duration) -> Result<(), ()> {
    let deadline = Instant::now() + timeout;
    loop {
        // SAFETY: signal zero only probes the existence of the owned process group.
        let status = unsafe { libc::kill(-process_group, 0) };
        if status != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(());
        }
        thread::sleep(PROCESS_POLL);
    }
}

fn parse_digest(bytes: &[u8]) -> Result<String, Stage1ProcessError> {
    if bytes.len() != 65 || bytes[64] != b'\n' {
        return Err(Stage1ProcessError::InvalidOutput);
    }
    let digest = &bytes[..64];
    if !digest
        .iter()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(Stage1ProcessError::InvalidOutput);
    }
    String::from_utf8(digest.to_vec()).map_err(|_| Stage1ProcessError::InvalidOutput)
}
