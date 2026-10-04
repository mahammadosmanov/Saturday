use crate::helper::manifest::{
    generate_run_id, FrozenCandidate, MAX_CHUNKS, MAX_OUTPUT_BYTES, MAX_PEAK_RSS_BYTES,
    MAX_PROCESS_COUNT, MAX_STDERR_BYTES, PROBE_TIMEOUT_MS, STARTUP_TIMEOUT_MS,
};
use crate::helper::protocol::{
    read_frame, write_frame, ChunkSequence, ProbeKind, ProtocolError, Request, RequestEnvelope,
    Response, ResponseEnvelope, PROTOCOL_VERSION,
};
use crate::helper::sandbox::{FixtureMode, LaunchPolicy, SandboxError};
use sha2::{Digest, Sha256};
use std::fs::{self, DirBuilder};
use std::io::{self, Read};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;

const READY_TIMEOUT: Duration = Duration::from_millis(STARTUP_TIMEOUT_MS);
const PROBE_TIMEOUT: Duration = Duration::from_millis(PROBE_TIMEOUT_MS);
const CANCEL_TIMEOUT: Duration = Duration::from_millis(250);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MONITOR_INTERVAL: Duration = Duration::from_millis(10);
const RESPONSE_QUEUE_CAPACITY: usize = MAX_CHUNKS as usize + 2;
const RESOURCE_OK: u8 = 0;
const RESOURCE_STDOUT: u8 = 1;
const RESOURCE_STDERR: u8 = 2;
const RESOURCE_RSS: u8 = 3;
const RESOURCE_PROCESSES: u8 = 4;
const RESOURCE_ACCOUNTING: u8 = 5;

#[derive(Default)]
struct ResourceState {
    violation: AtomicU8,
    peak_rss_bytes: AtomicU64,
    max_process_count: AtomicU32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessGroupSnapshot {
    pub pids: Vec<u32>,
    pub rss_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StderrDigest {
    pub bytes: u64,
    pub sha256: String,
    pub limit_exceeded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub output_bytes: u64,
    pub output_sha256: String,
    pub chunk_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelReceipt {
    pub acknowledgement_millis: u64,
    pub late_frames: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownReceipt {
    pub forced: bool,
    pub surviving_processes: u32,
    pub stderr: StderrDigest,
    pub peak_process_group_rss_bytes: u64,
    pub max_process_count: u32,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("sandbox launch policy failed: {0}")]
    Sandbox(#[from] SandboxError),
    #[error("protocol failure: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("process I/O failure: {0}")]
    Io(#[from] io::Error),
    #[error("helper handshake failed")]
    Handshake,
    #[error("helper response timed out")]
    Timeout,
    #[error("helper cancellation acknowledgement timed out")]
    CancelTimeout,
    #[error("helper response ownership failed")]
    Ownership,
    #[error("helper reported an error")]
    HelperError,
    #[error("process accounting failed: {0}")]
    Accounting(&'static str),
    #[error("fixed resource limit exceeded: {0}")]
    ResourceLimit(&'static str),
    #[error("worker thread failed")]
    Worker,
}

pub struct HelperGeneration {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    responses: Receiver<Result<ResponseEnvelope, ProtocolError>>,
    stdout_worker: Option<JoinHandle<()>>,
    stderr_worker: Option<JoinHandle<io::Result<StderrDigest>>>,
    monitor_worker: Option<JoinHandle<()>>,
    monitor_stop: Arc<AtomicBool>,
    resources: Arc<ResourceState>,
    process_group_id: i32,
    generation_id: String,
    cancelled_request_id: Option<u64>,
    late_frames: u32,
    next_request_id: u64,
}

impl HelperGeneration {
    pub fn launch(
        policy: &impl LaunchPolicy,
        candidate: &FrozenCandidate,
        runtime_root: &Path,
        mode: FixtureMode,
    ) -> Result<Self, ProcessError> {
        prepare_runtime_directory(runtime_root)?;
        let mut command = policy.build_command(candidate, runtime_root, mode)?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        #[cfg(feature = "fixture-helper")]
        let allow_fixture_descendants = matches!(
            mode,
            FixtureMode::Descendant | FixtureMode::ProcessFlood | FixtureMode::ProcessPulse
        );
        #[cfg(not(feature = "fixture-helper"))]
        let allow_fixture_descendants = false;
        // SAFETY: this child-only hook applies fixed kernel resource limits before exec.
        unsafe {
            command.pre_exec(move || {
                if !allow_fixture_descendants {
                    let process_limit = libc::rlimit {
                        rlim_cur: 1,
                        rlim_max: 1,
                    };
                    if libc::setrlimit(libc::RLIMIT_NPROC, &process_limit) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let process_group_id =
            i32::try_from(child.id()).map_err(|_| ProcessError::Accounting("PID conversion"))?;
        let stdin = child.stdin.take().ok_or(ProcessError::Handshake)?;
        let mut stdout = child.stdout.take().ok_or(ProcessError::Handshake)?;
        let stderr = child.stderr.take().ok_or(ProcessError::Handshake)?;
        let generation_id = generate_run_id().map_err(|_| ProcessError::Handshake)?;
        let resources = Arc::new(ResourceState::default());
        let monitor_stop = Arc::new(AtomicBool::new(false));
        let (sender, responses) = mpsc::sync_channel(RESPONSE_QUEUE_CAPACITY);
        let stdout_resources = Arc::clone(&resources);
        let stdout_worker = thread::spawn(move || loop {
            let response = read_frame::<_, ResponseEnvelope>(&mut stdout);
            let terminal = response.is_err();
            match sender.try_send(response) {
                Ok(()) if !terminal => {}
                Ok(()) | Err(TrySendError::Disconnected(_)) => break,
                Err(TrySendError::Full(_)) => {
                    mark_resource_violation(&stdout_resources, RESOURCE_STDOUT);
                    break;
                }
            }
        });
        let stderr_resources = Arc::clone(&resources);
        let stderr_worker = thread::spawn(move || {
            digest_reader_monitored(stderr, MAX_STDERR_BYTES, &stderr_resources)
        });
        let monitor_worker = Some(start_resource_monitor(
            process_group_id,
            Arc::clone(&resources),
            Arc::clone(&monitor_stop),
        ));
        let mut generation = Self {
            child: Some(child),
            stdin: Some(stdin),
            responses,
            stdout_worker: Some(stdout_worker),
            stderr_worker: Some(stderr_worker),
            monitor_worker,
            monitor_stop,
            resources,
            process_group_id,
            generation_id: generation_id.clone(),
            cancelled_request_id: None,
            late_frames: 0,
            next_request_id: 1,
        };
        let hello_id = generation.send(Request::Hello {
            manifest_sha256: candidate.manifest_sha256().to_owned(),
            run_id: candidate.manifest().run_id.clone(),
            generation_id: generation_id.clone(),
        })?;
        match generation.receive(READY_TIMEOUT)? {
            ResponseEnvelope {
                request_id,
                response:
                    Response::Ready {
                        helper_build_id,
                        run_id,
                        generation_id: response_generation_id,
                    },
                ..
            } if request_id == hello_id
                && helper_build_id == candidate.manifest().helper_build_id
                && run_id == candidate.manifest().run_id
                && response_generation_id == generation_id =>
            {
                generation.snapshot()?;
                Ok(generation)
            }
            _ => {
                let _ = generation.terminate_group();
                Err(ProcessError::Handshake)
            }
        }
    }

    pub fn process_group_id(&self) -> i32 {
        self.process_group_id
    }

    pub fn generation_id(&self) -> &str {
        &self.generation_id
    }

    pub fn snapshot(&self) -> Result<ProcessGroupSnapshot, ProcessError> {
        self.check_resource_violation()?;
        let snapshot = process_group_snapshot(self.process_group_id)?;
        observe_resource_snapshot(&self.resources, &snapshot);
        validate_snapshot(&snapshot)?;
        self.check_resource_violation()?;
        Ok(snapshot)
    }

    pub fn begin_probe(&mut self) -> Result<u64, ProcessError> {
        self.snapshot()?;
        let request_id = self.send(Request::Probe {
            probe: ProbeKind::Deterministic,
        })?;
        match self.receive(READY_TIMEOUT)? {
            ResponseEnvelope {
                request_id: response_id,
                response: Response::Started,
                ..
            } if response_id == request_id => Ok(request_id),
            _ => Err(ProcessError::Ownership),
        }
    }

    pub fn finish_probe(&mut self, request_id: u64) -> Result<ProbeResult, ProcessError> {
        let deadline = Instant::now() + PROBE_TIMEOUT;
        let mut sequence = ChunkSequence::default();
        let mut hasher = Sha256::new();
        let mut output_bytes = 0_u64;
        let mut chunk_count = 0_u32;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(ProcessError::Timeout)?;
            let envelope = self.receive(remaining)?;
            self.check_resource_violation()?;
            if envelope.request_id != request_id {
                return Err(ProcessError::Ownership);
            }
            match envelope.response {
                Response::Chunk {
                    sequence: index,
                    text,
                } => {
                    sequence.observe(index)?;
                    output_bytes = output_bytes
                        .checked_add(text.len() as u64)
                        .ok_or(ProcessError::Accounting("output byte overflow"))?;
                    if output_bytes > MAX_OUTPUT_BYTES {
                        return Err(ProcessError::ResourceLimit("aggregate output bytes"));
                    }
                    chunk_count = chunk_count
                        .checked_add(1)
                        .ok_or(ProcessError::Accounting("chunk count overflow"))?;
                    if chunk_count > MAX_CHUNKS {
                        return Err(ProcessError::ResourceLimit("chunk count"));
                    }
                    hasher.update(text.as_bytes());
                }
                Response::Complete {
                    output_bytes: reported_bytes,
                    output_sha256,
                } => {
                    self.snapshot()?;
                    let observed_sha256 = format_digest(hasher.finalize());
                    if reported_bytes != output_bytes || output_sha256 != observed_sha256 {
                        return Err(ProcessError::Ownership);
                    }
                    return Ok(ProbeResult {
                        output_bytes,
                        output_sha256,
                        chunk_count,
                    });
                }
                Response::Error { .. } => return Err(ProcessError::HelperError),
                _ => return Err(ProcessError::Ownership),
            }
        }
    }

    pub fn cancel(&mut self, target_request_id: u64) -> Result<CancelReceipt, ProcessError> {
        let started = Instant::now();
        self.send(Request::Cancel { target_request_id })?;
        match self.receive(CANCEL_TIMEOUT) {
            Ok(ResponseEnvelope {
                request_id,
                response: Response::Cancelled,
                ..
            }) if request_id == target_request_id => {
                let acknowledgement_millis = started.elapsed().as_millis() as u64;
                self.cancelled_request_id = Some(target_request_id);
                self.observe_cancel_window(started + CANCEL_TIMEOUT)?;
                Ok(CancelReceipt {
                    acknowledgement_millis,
                    late_frames: self.late_frames,
                })
            }
            Ok(_) => Err(ProcessError::Ownership),
            Err(ProcessError::Timeout) => {
                self.terminate_group()?;
                Err(ProcessError::CancelTimeout)
            }
            Err(error) => Err(error),
        }
    }

    pub fn shutdown(mut self) -> Result<ShutdownReceipt, ProcessError> {
        let _ = self.send(Request::Shutdown);
        let graceful = self.wait_for_direct_child(SHUTDOWN_TIMEOUT)?;
        let mut forced = !graceful;
        if !process_group_snapshot(self.process_group_id)?
            .pids
            .is_empty()
        {
            forced = true;
            self.terminate_group()?;
        }
        let surviving_processes = process_group_snapshot(self.process_group_id)?.pids.len() as u32;
        self.stop_monitor()?;
        let stderr = self.join_workers()?;
        self.drain_completed_responses()?;
        if stderr.limit_exceeded {
            return Err(ProcessError::ResourceLimit("stderr bytes"));
        }
        if self.late_frames != 0 {
            return Err(ProcessError::Ownership);
        }
        self.check_resource_violation()?;
        Ok(ShutdownReceipt {
            forced,
            surviving_processes,
            stderr,
            peak_process_group_rss_bytes: self.resources.peak_rss_bytes.load(Ordering::Acquire),
            max_process_count: self.resources.max_process_count.load(Ordering::Acquire),
        })
    }

    fn send(&mut self, request: Request) -> Result<u64, ProcessError> {
        let request_id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or(ProcessError::Ownership)?;
        let envelope = RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            request,
        };
        let stdin = self.stdin.as_mut().ok_or(ProcessError::Ownership)?;
        write_frame(stdin, &envelope)?;
        Ok(request_id)
    }

    fn receive(&mut self, timeout: Duration) -> Result<ResponseEnvelope, ProcessError> {
        let deadline = Instant::now() + timeout;
        loop {
            self.check_resource_violation()?;
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(ProcessError::Timeout)?;
            match self.responses.recv_timeout(remaining.min(MONITOR_INTERVAL)) {
                Ok(Ok(response)) => {
                    if self.cancelled_request_id == Some(response.request_id) {
                        self.late_frames = self.late_frames.saturating_add(1);
                        return Err(ProcessError::Ownership);
                    }
                    return Ok(response);
                }
                Ok(Err(error)) => {
                    self.check_resource_violation()?;
                    return Err(ProcessError::Protocol(error));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.check_resource_violation()?;
                    return Err(ProcessError::Handshake);
                }
            }
        }
    }

    fn observe_cancel_window(&mut self, deadline: Instant) -> Result<(), ProcessError> {
        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(());
            };
            match self.responses.recv_timeout(remaining) {
                Ok(Ok(response)) if self.cancelled_request_id == Some(response.request_id) => {
                    self.late_frames = self.late_frames.saturating_add(1);
                }
                Ok(Ok(_)) | Ok(Err(_)) => return Err(ProcessError::Ownership),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
    }

    fn drain_completed_responses(&mut self) -> Result<(), ProcessError> {
        loop {
            match self.responses.try_recv() {
                Ok(Ok(response)) if self.cancelled_request_id == Some(response.request_id) => {
                    self.late_frames = self.late_frames.saturating_add(1);
                }
                Ok(Err(ProtocolError::EndOfStream)) => {}
                Ok(Ok(_)) | Ok(Err(_)) => return Err(ProcessError::Ownership),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(()),
            }
        }
    }

    fn wait_for_direct_child(&mut self, timeout: Duration) -> Result<bool, ProcessError> {
        let deadline = Instant::now() + timeout;
        loop {
            let child = match self.child.as_mut() {
                Some(child) => child,
                None => return Ok(true),
            };
            if child.try_wait()?.is_some() {
                let mut child = self.child.take().expect("child checked above");
                let _ = child.wait();
                self.stdin.take();
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate_group(&mut self) -> Result<(), ProcessError> {
        self.stdin.take();
        signal_process_group(self.process_group_id, libc::SIGTERM)?;
        if !self.wait_for_direct_child(Duration::from_millis(100))? {
            signal_process_group(self.process_group_id, libc::SIGKILL)?;
            let _ = self.wait_for_direct_child(SHUTDOWN_TIMEOUT)?;
        }
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        while !process_group_snapshot(self.process_group_id)?
            .pids
            .is_empty()
        {
            if Instant::now() >= deadline {
                return Err(ProcessError::Accounting(
                    "process group survived termination",
                ));
            }
            let _ = signal_process_group(self.process_group_id, libc::SIGKILL);
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    fn join_workers(&mut self) -> Result<StderrDigest, ProcessError> {
        if let Some(worker) = self.stdout_worker.take() {
            worker.join().map_err(|_| ProcessError::Worker)?;
        }
        match self.stderr_worker.take() {
            Some(worker) => worker
                .join()
                .map_err(|_| ProcessError::Worker)?
                .map_err(ProcessError::Io),
            None => Ok(StderrDigest {
                bytes: 0,
                sha256: format_digest(Sha256::digest([])),
                limit_exceeded: false,
            }),
        }
    }

    fn stop_monitor(&mut self) -> Result<(), ProcessError> {
        self.monitor_stop.store(true, Ordering::Release);
        if let Some(worker) = self.monitor_worker.take() {
            worker.join().map_err(|_| ProcessError::Worker)?;
        }
        Ok(())
    }

    fn check_resource_violation(&self) -> Result<(), ProcessError> {
        match self.resources.violation.load(Ordering::Acquire) {
            RESOURCE_OK => Ok(()),
            RESOURCE_STDOUT => Err(ProcessError::ResourceLimit("response channel")),
            RESOURCE_STDERR => Err(ProcessError::ResourceLimit("stderr bytes")),
            RESOURCE_RSS => Err(ProcessError::ResourceLimit("resident memory")),
            RESOURCE_PROCESSES => Err(ProcessError::ResourceLimit("process count")),
            RESOURCE_ACCOUNTING => Err(ProcessError::Accounting("resource monitor")),
            _ => Err(ProcessError::Accounting("unknown resource violation")),
        }
    }
}

impl Drop for HelperGeneration {
    fn drop(&mut self) {
        let _ = self.terminate_group();
        let _ = self.stop_monitor();
        let _ = self.join_workers();
    }
}

fn start_resource_monitor(
    process_group_id: i32,
    resources: Arc<ResourceState>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            match process_group_snapshot(process_group_id) {
                Ok(snapshot) => {
                    observe_resource_snapshot(&resources, &snapshot);
                    if snapshot.pids.len() as u32 > MAX_PROCESS_COUNT {
                        mark_resource_violation(&resources, RESOURCE_PROCESSES);
                    }
                    if snapshot.rss_bytes > MAX_PEAK_RSS_BYTES {
                        mark_resource_violation(&resources, RESOURCE_RSS);
                    }
                }
                Err(_) => mark_resource_violation(&resources, RESOURCE_ACCOUNTING),
            }
            if resources.violation.load(Ordering::Acquire) != RESOURCE_OK {
                let _ = signal_process_group(process_group_id, libc::SIGKILL);
                break;
            }
            thread::sleep(MONITOR_INTERVAL);
        }
    })
}

fn observe_resource_snapshot(resources: &ResourceState, snapshot: &ProcessGroupSnapshot) {
    resources
        .peak_rss_bytes
        .fetch_max(snapshot.rss_bytes, Ordering::AcqRel);
    resources
        .max_process_count
        .fetch_max(snapshot.pids.len() as u32, Ordering::AcqRel);
}

fn mark_resource_violation(resources: &ResourceState, code: u8) {
    let _ = resources.violation.compare_exchange(
        RESOURCE_OK,
        code,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

fn prepare_runtime_directory(root: &Path) -> Result<(), ProcessError> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ProcessError::Accounting("invalid runtime root"));
    }
    for name in ["home", "tmp"] {
        let path = root.join(name);
        if !path.exists() {
            let mut builder = DirBuilder::new();
            builder.mode(0o700).create(&path)?;
        }
    }
    Ok(())
}

pub fn process_group_snapshot(process_group_id: i32) -> Result<ProcessGroupSnapshot, ProcessError> {
    let output = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,pgid=,rss="])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C")
        .output()?;
    if !output.status.success() {
        return Err(ProcessError::Accounting("ps failed"));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| ProcessError::Accounting("ps output is not UTF-8"))?;
    parse_process_group_rows(text, process_group_id)
}

pub fn parse_process_group_rows(
    rows: &str,
    process_group_id: i32,
) -> Result<ProcessGroupSnapshot, ProcessError> {
    let mut pids = Vec::new();
    let mut rss_bytes = 0_u64;
    for row in rows.lines().filter(|line| !line.trim().is_empty()) {
        let fields = row.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 4 {
            return Err(ProcessError::Accounting("malformed ps row"));
        }
        let pid = fields[0]
            .parse::<u32>()
            .map_err(|_| ProcessError::Accounting("invalid PID"))?;
        let _parent_pid = fields[1]
            .parse::<u32>()
            .map_err(|_| ProcessError::Accounting("invalid parent PID"))?;
        let pgid = fields[2]
            .parse::<i32>()
            .map_err(|_| ProcessError::Accounting("invalid process group"))?;
        let rss_kib = fields[3]
            .parse::<u64>()
            .map_err(|_| ProcessError::Accounting("invalid RSS"))?;
        if pgid == process_group_id {
            pids.push(pid);
            rss_bytes = rss_bytes
                .checked_add(
                    rss_kib
                        .checked_mul(1_024)
                        .ok_or(ProcessError::Accounting("RSS overflow"))?,
                )
                .ok_or(ProcessError::Accounting("RSS overflow"))?;
        }
    }
    pids.sort_unstable();
    Ok(ProcessGroupSnapshot { pids, rss_bytes })
}

pub fn digest_reader(reader: impl Read, maximum_bytes: u64) -> io::Result<StderrDigest> {
    digest_reader_monitored(reader, maximum_bytes, &ResourceState::default())
}

fn digest_reader_monitored(
    mut reader: impl Read,
    maximum_bytes: u64,
    resources: &ResourceState,
) -> io::Result<StderrDigest> {
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("stderr byte count overflow"))?;
        hasher.update(&buffer[..count]);
        if bytes > maximum_bytes {
            mark_resource_violation(resources, RESOURCE_STDERR);
            break;
        }
    }
    Ok(StderrDigest {
        bytes,
        sha256: format_digest(hasher.finalize()),
        limit_exceeded: bytes > maximum_bytes,
    })
}

fn validate_snapshot(snapshot: &ProcessGroupSnapshot) -> Result<(), ProcessError> {
    if snapshot.pids.len() as u32 > MAX_PROCESS_COUNT {
        return Err(ProcessError::ResourceLimit("process count"));
    }
    if snapshot.rss_bytes > MAX_PEAK_RSS_BYTES {
        return Err(ProcessError::ResourceLimit("process-group RSS"));
    }
    Ok(())
}

fn signal_process_group(process_group_id: i32, signal: i32) -> Result<(), ProcessError> {
    // SAFETY: `killpg` is called with a positive process-group ID created by this owner and a
    // constant POSIX signal. No pointers cross the FFI boundary.
    let result = unsafe { libc::killpg(process_group_id, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(ProcessError::Io(error))
    }
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

    #[test]
    fn parses_and_aggregates_only_requested_process_group() {
        let rows = "100 1 100 2048\n101 100 100 4096\n200 1 200 9999\n";
        let snapshot = parse_process_group_rows(rows, 100).unwrap();
        assert_eq!(snapshot.pids, vec![100, 101]);
        assert_eq!(snapshot.rss_bytes, 6_144 * 1_024);
    }

    #[test]
    fn process_accounting_fails_closed_on_malformed_rows() {
        assert!(matches!(
            parse_process_group_rows("not process data", 100),
            Err(ProcessError::Accounting(_))
        ));
    }

    #[test]
    fn stderr_digest_retains_only_count_and_hash() {
        let digest = digest_reader(&b"secret-shaped fixture output"[..], 1_024).unwrap();
        assert_eq!(digest.bytes, 28);
        assert_eq!(digest.sha256.len(), 64);
        assert!(!digest.limit_exceeded);
    }
}
