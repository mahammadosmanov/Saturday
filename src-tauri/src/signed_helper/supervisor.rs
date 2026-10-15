use crate::helper::manifest::{generate_run_id, MAX_CHUNKS};
use crate::helper::process::{digest_reader, process_group_snapshot};
use crate::helper::protocol::{
    read_frame, write_frame, ProbeKind, Request, RequestEnvelope, Response, ResponseEnvelope,
    PROTOCOL_VERSION,
};
use crate::helper::sandbox::DENY_NETWORK_PROFILE;
use crate::platform::code_identity::process_matches_current_team;
use crate::signed_helper::identity::{read_identity, verify_helper_file, HelperIdentityError};
use sha2::{Digest, Sha256};
use std::fs::{self, DirBuilder};
use std::io::{self, BufReader, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;

const HELPER_NAME: &str = "saturday-runtime-helper";
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_STDERR_BYTES: u64 = 65_536;
const RESPONSE_QUEUE_CAPACITY: usize = MAX_CHUNKS as usize + 2;

#[derive(Debug, Error)]
pub enum SignedHelperError {
    #[error("bundle layout is invalid")]
    Layout,
    #[error("helper identity is invalid")]
    Identity,
    #[error("helper peer identity is unauthorized")]
    PeerIdentity,
    #[error("helper launch failed")]
    Launch,
    #[error("helper protocol failed")]
    Protocol,
    #[error("helper deadline expired")]
    Timeout,
    #[error("helper cleanup failed")]
    Cleanup,
}

impl SignedHelperError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Layout => "layout",
            Self::Identity => "identity",
            Self::PeerIdentity => "peer_identity",
            Self::Launch => "launch",
            Self::Protocol => "protocol",
            Self::Timeout => "timeout",
            Self::Cleanup => "cleanup",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BundlePaths {
    helper: PathBuf,
    identity: PathBuf,
}

pub fn run_bundled_self_test() -> Result<(), SignedHelperError> {
    let executable = std::env::current_exe().map_err(|_| SignedHelperError::Layout)?;
    let paths = resolve_bundle_paths(&executable)?;
    let identity = read_identity(&paths.identity).map_err(map_identity_error)?;
    let verified_file = verify_helper_file(&paths.helper, &identity).map_err(map_identity_error)?;
    if !matches!(
        process_matches_current_team(std::process::id() as i32),
        Ok(true)
    ) {
        return Err(SignedHelperError::PeerIdentity);
    }

    let mut runtime = RuntimeRoot::create()?;
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .arg("-p")
        .arg(DENY_NETWORK_PROFILE)
        .arg(&paths.helper)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("HOME", &runtime.home)
        .env("TMPDIR", &runtime.temporary)
        .process_group(0);
    // SAFETY: this closure runs after fork and before exec, calls only async-signal-safe
    // setrlimit, and returns the OS error directly.
    unsafe {
        command.pre_exec(|| {
            let process_limit = libc::rlimit {
                rlim_cur: 1,
                rlim_max: 1,
            };
            if libc::setrlimit(libc::RLIMIT_NPROC, &process_limit) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|_| SignedHelperError::Launch)?;
    drop(verified_file);
    let process_group = child.id() as i32;
    let stdin = child.stdin.take().ok_or(SignedHelperError::Launch)?;
    let stdout = child.stdout.take().ok_or(SignedHelperError::Launch)?;
    let stderr = child.stderr.take().ok_or(SignedHelperError::Launch)?;
    let (sender, responses) = mpsc::sync_channel(RESPONSE_QUEUE_CAPACITY);
    let stdout_overflow = Arc::new(AtomicBool::new(false));
    let worker_overflow = Arc::clone(&stdout_overflow);
    let stdout_worker = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let frame = read_frame::<_, ResponseEnvelope>(&mut reader);
            let terminal = frame.is_err();
            match sender.try_send(frame) {
                Ok(()) if terminal => break,
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    worker_overflow.store(true, Ordering::Release);
                    break;
                }
                Err(TrySendError::Disconnected(_)) => break,
            }
        }
    });
    let stderr_worker = thread::spawn(move || digest_reader(stderr, MAX_STDERR_BYTES));
    let mut owned = OwnedSignedChild {
        child,
        stdin: Some(stdin),
        responses,
        stdout_worker: Some(stdout_worker),
        stderr_worker: Some(stderr_worker),
        process_group,
        stdout_overflow,
    };

    wait_for_peer_identity(&mut owned)?;
    let run_id = generate_run_id().map_err(|_| SignedHelperError::Protocol)?;
    let generation_id = generate_run_id().map_err(|_| SignedHelperError::Protocol)?;
    owned.send(
        1,
        Request::Hello {
            manifest_sha256: identity.binary_sha256.clone(),
            run_id: run_id.clone(),
            generation_id: generation_id.clone(),
        },
    )?;
    let ready = owned.receive(RESPONSE_TIMEOUT)?;
    match ready.response {
        Response::Ready {
            helper_build_id,
            run_id: echoed_run,
            generation_id: echoed_generation,
        } if ready.request_id == 1
            && helper_build_id == identity.source_generation_sha256
            && echoed_run == run_id
            && echoed_generation == generation_id => {}
        _ => return Err(SignedHelperError::Protocol),
    }

    owned.send(
        2,
        Request::Probe {
            probe: ProbeKind::Health,
        },
    )?;
    expect_response(&mut owned, 2, |response| {
        matches!(response, Response::Started)
    })?;
    let chunk = owned.receive(RESPONSE_TIMEOUT)?;
    let output = match chunk.response {
        Response::Chunk { sequence: 0, text } if chunk.request_id == 2 => text,
        _ => return Err(SignedHelperError::Protocol),
    };
    let complete = owned.receive(RESPONSE_TIMEOUT)?;
    match complete.response {
        Response::Complete {
            output_bytes,
            output_sha256,
        } if complete.request_id == 2
            && output == "healthy"
            && output_bytes == output.len() as u64
            && output_sha256 == format!("{:x}", Sha256::digest(output.as_bytes())) => {}
        _ => return Err(SignedHelperError::Protocol),
    }

    owned.send(
        3,
        Request::Probe {
            probe: ProbeKind::Cancellation,
        },
    )?;
    expect_response(&mut owned, 3, |response| {
        matches!(response, Response::Started)
    })?;
    owned.send(
        4,
        Request::Cancel {
            target_request_id: 3,
        },
    )?;
    expect_response(&mut owned, 3, |response| {
        matches!(response, Response::Cancelled)
    })?;
    owned.send(5, Request::Shutdown)?;
    owned.close_stdin();
    owned.finish()?;
    runtime.remove_verified()
}

fn expect_response(
    child: &mut OwnedSignedChild,
    request_id: u64,
    predicate: impl FnOnce(&Response) -> bool,
) -> Result<(), SignedHelperError> {
    let response = child.receive(RESPONSE_TIMEOUT)?;
    if response.request_id == request_id && predicate(&response.response) {
        Ok(())
    } else {
        Err(SignedHelperError::Protocol)
    }
}

fn resolve_bundle_paths(executable: &Path) -> Result<BundlePaths, SignedHelperError> {
    let executable = executable
        .canonicalize()
        .map_err(|_| SignedHelperError::Layout)?;
    let macos = executable.parent().ok_or(SignedHelperError::Layout)?;
    if macos.file_name().and_then(|value| value.to_str()) != Some("MacOS") {
        return Err(SignedHelperError::Layout);
    }
    let contents = macos.parent().ok_or(SignedHelperError::Layout)?;
    if contents.file_name().and_then(|value| value.to_str()) != Some("Contents") {
        return Err(SignedHelperError::Layout);
    }
    let app = contents.parent().ok_or(SignedHelperError::Layout)?;
    if app.extension().and_then(|value| value.to_str()) != Some("app") {
        return Err(SignedHelperError::Layout);
    }
    Ok(BundlePaths {
        helper: macos.join(HELPER_NAME),
        identity: contents
            .join("Resources")
            .join("saturday-resources")
            .join("helper-identity.json"),
    })
}

fn wait_for_peer_identity(child: &mut OwnedSignedChild) -> Result<(), SignedHelperError> {
    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    loop {
        if matches!(
            process_matches_current_team(child.child.id() as i32),
            Ok(true)
        ) {
            return Ok(());
        }
        if child
            .child
            .try_wait()
            .map_err(|_| SignedHelperError::Launch)?
            .is_some()
        {
            return Err(SignedHelperError::PeerIdentity);
        }
        if Instant::now() >= deadline {
            return Err(SignedHelperError::PeerIdentity);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn map_identity_error(_error: HelperIdentityError) -> SignedHelperError {
    SignedHelperError::Identity
}

struct OwnedSignedChild {
    child: Child,
    stdin: Option<ChildStdin>,
    responses: Receiver<Result<ResponseEnvelope, crate::helper::protocol::ProtocolError>>,
    stdout_worker: Option<JoinHandle<()>>,
    stderr_worker: Option<JoinHandle<io::Result<crate::helper::process::StderrDigest>>>,
    process_group: i32,
    stdout_overflow: Arc<AtomicBool>,
}

impl OwnedSignedChild {
    fn send(&mut self, request_id: u64, request: Request) -> Result<(), SignedHelperError> {
        let stdin = self.stdin.as_mut().ok_or(SignedHelperError::Protocol)?;
        write_frame(
            stdin,
            &RequestEnvelope {
                protocol_version: PROTOCOL_VERSION,
                request_id,
                request,
            },
        )
        .map_err(|_| SignedHelperError::Protocol)?;
        stdin.flush().map_err(|_| SignedHelperError::Protocol)
    }

    fn receive(&self, timeout: Duration) -> Result<ResponseEnvelope, SignedHelperError> {
        if self.stdout_overflow.load(Ordering::Acquire) {
            return Err(SignedHelperError::Protocol);
        }
        let response = receive_response(&self.responses, timeout)?;
        if self.stdout_overflow.load(Ordering::Acquire) {
            return Err(SignedHelperError::Protocol);
        }
        Ok(response)
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
    }

    fn finish(mut self) -> Result<(), SignedHelperError> {
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        let exit_status = loop {
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|_| SignedHelperError::Cleanup)?
            {
                break status;
            }
            if Instant::now() >= deadline {
                return Err(SignedHelperError::Timeout);
            }
            thread::sleep(Duration::from_millis(10));
        };
        self.join_workers()?;
        if !exit_status.success() {
            return Err(SignedHelperError::Protocol);
        }
        let surviving =
            process_group_snapshot(self.process_group).map_err(|_| SignedHelperError::Cleanup)?;
        if !surviving.pids.is_empty() {
            return Err(SignedHelperError::Cleanup);
        }
        Ok(())
    }

    fn join_workers(&mut self) -> Result<(), SignedHelperError> {
        if let Some(worker) = self.stdout_worker.take() {
            worker.join().map_err(|_| SignedHelperError::Cleanup)?;
        }
        let stderr = self
            .stderr_worker
            .take()
            .ok_or(SignedHelperError::Cleanup)?
            .join()
            .map_err(|_| SignedHelperError::Cleanup)?
            .map_err(|_| SignedHelperError::Cleanup)?;
        if stderr.limit_exceeded || stderr.bytes != 0 {
            return Err(SignedHelperError::Protocol);
        }
        if self.stdout_overflow.load(Ordering::Acquire) {
            return Err(SignedHelperError::Protocol);
        }
        ensure_no_late_responses(&self.responses)
    }

    fn terminate(&mut self) {
        self.close_stdin();
        // SAFETY: a negative pid targets only the owned process group.
        unsafe {
            libc::kill(-self.process_group, libc::SIGKILL);
        }
        let _ = self.child.wait();
        let _ = self.join_workers();
    }
}

fn receive_response(
    responses: &Receiver<Result<ResponseEnvelope, crate::helper::protocol::ProtocolError>>,
    timeout: Duration,
) -> Result<ResponseEnvelope, SignedHelperError> {
    match responses.recv_timeout(timeout) {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(_)) => Err(SignedHelperError::Protocol),
        Err(RecvTimeoutError::Timeout) => Err(SignedHelperError::Timeout),
        Err(RecvTimeoutError::Disconnected) => Err(SignedHelperError::Protocol),
    }
}

fn ensure_no_late_responses(
    responses: &Receiver<Result<ResponseEnvelope, crate::helper::protocol::ProtocolError>>,
) -> Result<(), SignedHelperError> {
    while let Ok(frame) = responses.try_recv() {
        match frame {
            Err(crate::helper::protocol::ProtocolError::EndOfStream) => {}
            _ => return Err(SignedHelperError::Protocol),
        }
    }
    Ok(())
}

impl Drop for OwnedSignedChild {
    fn drop(&mut self) {
        if self.stdout_worker.is_some() || self.stderr_worker.is_some() {
            self.terminate();
        }
    }
}

struct RuntimeRoot {
    root: PathBuf,
    home: PathBuf,
    temporary: PathBuf,
    cleaned: bool,
}

impl RuntimeRoot {
    fn create() -> Result<Self, SignedHelperError> {
        let root = std::env::temp_dir().join(format!(
            "saturday-signed-helper-{}",
            generate_run_id().map_err(|_| SignedHelperError::Launch)?
        ));
        Self::create_at(root, |path| DirBuilder::new().mode(0o700).create(path))
    }

    fn create_at(
        root: PathBuf,
        mut create_child: impl FnMut(&Path) -> io::Result<()>,
    ) -> Result<Self, SignedHelperError> {
        DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .map_err(|_| SignedHelperError::Launch)?;
        let runtime = Self {
            home: root.join("home"),
            temporary: root.join("tmp"),
            root,
            cleaned: false,
        };
        for path in [&runtime.home, &runtime.temporary] {
            create_child(path).map_err(|_| SignedHelperError::Launch)?;
        }
        Ok(runtime)
    }

    fn remove_verified(&mut self) -> Result<(), SignedHelperError> {
        fs::remove_dir_all(&self.root).map_err(|_| SignedHelperError::Cleanup)?;
        match fs::symlink_metadata(&self.root) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.cleaned = true;
                Ok(())
            }
            _ => Err(SignedHelperError::Cleanup),
        }
    }
}

impl Drop for RuntimeRoot {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn bundle_paths_require_standard_app_layout() {
        let root =
            std::env::temp_dir().join(format!("saturday-bundle-layout-{}", std::process::id()));
        let executable = root
            .join("Saturday.app")
            .join("Contents")
            .join("MacOS")
            .join("saturday");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, b"app").unwrap();
        let paths = resolve_bundle_paths(&executable).unwrap();
        assert!(paths
            .helper
            .ends_with("Contents/MacOS/saturday-runtime-helper"));
        assert!(paths
            .identity
            .ends_with("Contents/Resources/saturday-resources/helper-identity.json"));
        fs::remove_dir_all(root).unwrap();
        assert!(resolve_bundle_paths(Path::new("/bin/sh")).is_err());
    }

    #[test]
    fn errors_expose_only_fixed_codes() {
        assert_eq!(SignedHelperError::Identity.code(), "identity");
        assert_eq!(SignedHelperError::PeerIdentity.code(), "peer_identity");
    }

    #[test]
    fn response_deadlines_and_late_frames_fail_closed() {
        let (_sender, receiver) = mpsc::sync_channel::<
            Result<ResponseEnvelope, crate::helper::protocol::ProtocolError>,
        >(1);
        assert!(matches!(
            receive_response(&receiver, Duration::from_millis(1)),
            Err(SignedHelperError::Timeout)
        ));

        let (sender, receiver) = mpsc::sync_channel(1);
        sender
            .send(Ok(ResponseEnvelope {
                protocol_version: PROTOCOL_VERSION,
                request_id: 99,
                response: Response::Started,
            }))
            .unwrap();
        assert!(matches!(
            ensure_no_late_responses(&receiver),
            Err(SignedHelperError::Protocol)
        ));

        let (sender, receiver) = mpsc::sync_channel(1);
        sender
            .send(Err(crate::helper::protocol::ProtocolError::InvalidJson))
            .unwrap();
        assert!(matches!(
            ensure_no_late_responses(&receiver),
            Err(SignedHelperError::Protocol)
        ));

        let (sender, receiver) = mpsc::sync_channel(1);
        sender
            .send(Err(crate::helper::protocol::ProtocolError::EndOfStream))
            .unwrap();
        ensure_no_late_responses(&receiver).unwrap();
    }

    fn runtime_root_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "saturday-runtime-root-test-{}",
            generate_run_id().unwrap()
        ))
    }

    #[test]
    fn runtime_root_removes_partial_tree_when_child_creation_fails() {
        for failing_child in 0..2 {
            let root = runtime_root_path();
            let mut child_index = 0;
            let result = RuntimeRoot::create_at(root.clone(), |path| {
                let current = child_index;
                child_index += 1;
                if current == failing_child {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected child creation failure",
                    ));
                }
                DirBuilder::new().mode(0o700).create(path)
            });

            assert!(matches!(result, Err(SignedHelperError::Launch)));
            assert_eq!(child_index, failing_child + 1);
            assert!(!root.exists());
        }
    }

    #[test]
    fn runtime_root_creates_private_directories_and_explicitly_cleans_up() {
        let mut runtime = RuntimeRoot::create_at(runtime_root_path(), |path| {
            DirBuilder::new().mode(0o700).create(path)
        })
        .unwrap();

        for path in [&runtime.root, &runtime.home, &runtime.temporary] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }

        let root = runtime.root.clone();
        runtime.remove_verified().unwrap();
        assert!(!root.exists());
    }
}
