use crate::helper::evidence::{BoundaryMeasurements, CompletedBoundaryRun};
use crate::helper::manifest::FrozenCandidate;
use crate::helper::process::{HelperGeneration, ProbeResult, ProcessError, ProcessGroupSnapshot};
use crate::helper::sandbox::{FixtureMode, MacOsDenyNetwork, SandboxError};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, DirBuilder};
use std::io;
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("helper lifecycle failed: {0}")]
    Process(#[from] ProcessError),
    #[error("sandbox policy unavailable: {0}")]
    Sandbox(#[from] SandboxError),
    #[error("harness filesystem failure: {0}")]
    Io(#[from] io::Error),
    #[error("harness observation failed: {0}")]
    Observation(&'static str),
}

#[derive(Default)]
struct Measurements {
    generations: BTreeMap<String, String>,
    peak_rss_bytes: u64,
    max_process_count: u32,
    max_output_bytes: u64,
    max_chunks: u32,
    max_stderr_bytes: u64,
    surviving_processes: u32,
}

struct WorkspaceGuard {
    root: PathBuf,
    armed: bool,
}

impl WorkspaceGuard {
    fn create(root: &Path) -> Result<Self, HarnessError> {
        let mut builder = DirBuilder::new();
        builder.mode(0o700).create(root)?;
        Ok(Self {
            root: root.to_path_buf(),
            armed: true,
        })
    }

    fn scenario(&self, name: &str) -> Result<PathBuf, HarnessError> {
        let path = self.root.join(name);
        let mut builder = DirBuilder::new();
        builder.mode(0o700).create(&path)?;
        Ok(path)
    }

    fn remove_scenario(&self, path: &Path) -> Result<(), HarnessError> {
        fs::remove_dir_all(path)?;
        require_absent(path)
    }

    fn finish(mut self) -> Result<(), HarnessError> {
        fs::remove_dir_all(&self.root)?;
        require_absent(&self.root)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for WorkspaceGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

pub fn run_fixture_boundary(
    candidate: &FrozenCandidate,
    workspace_root: &Path,
) -> Result<CompletedBoundaryRun, HarnessError> {
    let policy = MacOsDenyNetwork::new()?;
    let workspace = WorkspaceGuard::create(workspace_root)?;
    let mut measured = Measurements::default();

    let normal_root = workspace.scenario("normal")?;
    let mut normal = launch_observed(
        &policy,
        candidate,
        &normal_root,
        FixtureMode::Normal,
        "normal",
        &mut measured,
    )?;
    let normal_id = normal.begin_probe()?;
    let normal_result = normal.finish_probe(normal_id)?;
    require_probe(&normal_result, "fixture-output")?;
    observe_probe(&normal_result, &mut measured);
    let normal_shutdown = normal.shutdown()?;
    observe_shutdown(&normal_shutdown, &mut measured);
    workspace.remove_scenario(&normal_root)?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let network_address = listener.local_addr()?;
    let control = TcpStream::connect(network_address)?;
    let (accepted, _) = listener.accept()?;
    drop(control);
    drop(accepted);
    let network_root = workspace.scenario("network")?;
    let mut network = launch_observed(
        &policy,
        candidate,
        &network_root,
        FixtureMode::NetworkAttempt(network_address.port()),
        "network",
        &mut measured,
    )?;
    let network_id = network.begin_probe()?;
    let network_result = network.finish_probe(network_id)?;
    require_probe(&network_result, "network-denied-policy")?;
    observe_probe(&network_result, &mut measured);
    let network_shutdown = network.shutdown()?;
    observe_shutdown(&network_shutdown, &mut measured);
    workspace.remove_scenario(&network_root)?;

    let cancel_root = workspace.scenario("cancel")?;
    let mut cancel = launch_observed(
        &policy,
        candidate,
        &cancel_root,
        FixtureMode::CancelDelay,
        "cancel",
        &mut measured,
    )?;
    let cancel_id = cancel.begin_probe()?;
    let cancellation = cancel.cancel(cancel_id)?;
    let cancel_shutdown = cancel.shutdown()?;
    observe_shutdown(&cancel_shutdown, &mut measured);
    workspace.remove_scenario(&cancel_root)?;

    workspace.finish()?;
    Ok(CompletedBoundaryRun::from_observations(
        candidate.manifest().run_id.clone(),
        measured.generations,
        BoundaryMeasurements {
            handshake_ready: true,
            probes_started: 3,
            probes_completed: 2,
            network_attempt_denied: true,
            cancel_acknowledged_ms: Some(cancellation.acknowledgement_millis),
            late_frames: cancellation.late_frames,
            surviving_processes: measured.surviving_processes,
            peak_process_group_rss_bytes: measured.peak_rss_bytes,
            max_process_count: measured.max_process_count,
            max_output_bytes: measured.max_output_bytes,
            max_chunks: measured.max_chunks,
            max_stderr_bytes: measured.max_stderr_bytes,
            temporary_root_removed: true,
        },
    ))
}

fn launch_observed(
    policy: &MacOsDenyNetwork,
    candidate: &FrozenCandidate,
    runtime_root: &Path,
    mode: FixtureMode,
    scenario: &str,
    measured: &mut Measurements,
) -> Result<HelperGeneration, HarnessError> {
    let generation = HelperGeneration::launch(policy, candidate, runtime_root, mode)?;
    let snapshot = generation.snapshot()?;
    observe_snapshot(&snapshot, measured);
    if measured
        .generations
        .insert(scenario.to_owned(), generation.generation_id().to_owned())
        .is_some()
    {
        return Err(HarnessError::Observation("duplicate scenario"));
    }
    Ok(generation)
}

fn observe_snapshot(snapshot: &ProcessGroupSnapshot, measured: &mut Measurements) {
    measured.peak_rss_bytes = measured.peak_rss_bytes.max(snapshot.rss_bytes);
    measured.max_process_count = measured.max_process_count.max(snapshot.pids.len() as u32);
}

fn observe_probe(probe: &ProbeResult, measured: &mut Measurements) {
    measured.max_output_bytes = measured.max_output_bytes.max(probe.output_bytes);
    measured.max_chunks = measured.max_chunks.max(probe.chunk_count);
}

fn observe_shutdown(
    shutdown: &crate::helper::process::ShutdownReceipt,
    measured: &mut Measurements,
) {
    measured.surviving_processes = measured
        .surviving_processes
        .saturating_add(shutdown.surviving_processes);
    measured.max_stderr_bytes = measured.max_stderr_bytes.max(shutdown.stderr.bytes);
    measured.peak_rss_bytes = measured
        .peak_rss_bytes
        .max(shutdown.peak_process_group_rss_bytes);
    measured.max_process_count = measured.max_process_count.max(shutdown.max_process_count);
}

fn require_probe(probe: &ProbeResult, expected: &str) -> Result<(), HarnessError> {
    let expected_hash = format_digest(Sha256::digest(expected.as_bytes()));
    if probe.output_bytes != expected.len() as u64
        || probe.output_sha256 != expected_hash
        || probe.chunk_count != 1
    {
        return Err(HarnessError::Observation(
            "unexpected fixture probe receipt",
        ));
    }
    Ok(())
}

fn require_absent(path: &Path) -> Result<(), HarnessError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(HarnessError::Observation("temporary root survived removal")),
        Err(error) => Err(HarnessError::Io(error)),
    }
}

fn format_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
