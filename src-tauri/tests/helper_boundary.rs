use jarvis_dashboard::helper::evidence::{publish_evidence, verify_evidence_dir};
use jarvis_dashboard::helper::harness::run_fixture_boundary;
use jarvis_dashboard::helper::manifest::{
    freeze_candidate, generate_run_id, sha256_file, CandidateManifestV1, CANCEL_ACK_TIMEOUT_MS,
    EVIDENCE_SCHEMA, LAUNCHER_POLICY_ID, MANIFEST_SCHEMA, MAX_CHUNKS, MAX_OUTPUT_BYTES,
    MAX_PEAK_RSS_BYTES, MAX_PROCESS_COUNT, MAX_STDERR_BYTES, PROBE_TIMEOUT_MS, SHUTDOWN_TIMEOUT_MS,
    STARTUP_TIMEOUT_MS,
};
use jarvis_dashboard::helper::process::{process_group_snapshot, HelperGeneration, ProcessError};
use jarvis_dashboard::helper::protocol::PROTOCOL_VERSION;
use jarvis_dashboard::helper::sandbox::{FixtureMode, MacOsDenyNetwork};
use sha2::{Digest, Sha256};
use std::fs::{self, DirBuilder};
use std::net::{TcpListener, TcpStream};
use std::ops::Deref;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

struct PrivateRoot {
    path: PathBuf,
}

impl Deref for PrivateRoot {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for PrivateRoot {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for PrivateRoot {
    fn drop(&mut self) {
        cleanup_path(&self.path);
    }
}

fn private_dir(label: &str) -> PrivateRoot {
    let path = private_path(label);
    let mut builder = DirBuilder::new();
    builder.mode(0o700).create(&path).unwrap();
    PrivateRoot { path }
}

fn private_path(label: &str) -> PathBuf {
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).unwrap();
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = std::env::temp_dir().join(format!("jarvis-boundary-{label}-{suffix}"));
    path
}

fn frozen_fixture() -> (
    PrivateRoot,
    jarvis_dashboard::helper::manifest::FrozenCandidate,
) {
    let root = private_dir("candidate");
    let destination = root.join("jarvis-helper-fixture");
    fs::copy(env!("CARGO_BIN_EXE_jarvis-helper-fixture"), &destination).unwrap();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o500)).unwrap();
    let metadata = fs::metadata(&destination).unwrap();
    let manifest = CandidateManifestV1 {
        schema: MANIFEST_SCHEMA.to_owned(),
        protocol_version: PROTOCOL_VERSION,
        run_id: generate_run_id().unwrap(),
        helper_build_id: "fixture-1".to_owned(),
        executable_relative_path: "jarvis-helper-fixture".to_owned(),
        executable_size_bytes: metadata.len(),
        executable_sha256: sha256_file(&destination).unwrap(),
        platform: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        cancel_ack_timeout_ms: CANCEL_ACK_TIMEOUT_MS,
        shutdown_timeout_ms: SHUTDOWN_TIMEOUT_MS,
        startup_timeout_ms: STARTUP_TIMEOUT_MS,
        probe_timeout_ms: PROBE_TIMEOUT_MS,
        max_output_bytes: MAX_OUTPUT_BYTES,
        max_chunks: MAX_CHUNKS,
        max_stderr_bytes: MAX_STDERR_BYTES,
        max_process_count: MAX_PROCESS_COUNT,
        max_peak_rss_bytes: MAX_PEAK_RSS_BYTES,
        launcher_policy_id: LAUNCHER_POLICY_ID.to_owned(),
        evidence_schema: EVIDENCE_SCHEMA.to_owned(),
    };
    let frozen = freeze_candidate(&root, manifest).unwrap();
    (root, frozen)
}

fn launch(
    candidate: &jarvis_dashboard::helper::manifest::FrozenCandidate,
    mode: FixtureMode,
) -> (PrivateRoot, HelperGeneration) {
    let runtime = private_dir("runtime");
    let generation =
        HelperGeneration::launch(&MacOsDenyNetwork::new().unwrap(), candidate, &runtime, mode)
            .unwrap();
    (runtime, generation)
}

fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn remove(path: impl AsRef<Path>) {
    cleanup_path(path.as_ref());
}

fn cleanup_path(path: &Path) {
    if !path.exists() {
        return;
    }
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    let _ = fs::remove_dir_all(path);
}

#[test]
fn private_roots_are_removed_during_unwind() {
    let path = private_path("unwind");
    let captured = path.clone();
    let result = std::panic::catch_unwind(move || {
        let root = private_dir("unwind-owned");
        fs::rename(&*root, &captured).unwrap();
        let mut moved = root;
        moved.path = captured;
        panic!("injected failure");
    });
    assert!(result.is_err());
    assert!(!path.exists());
}

#[test]
fn normal_probe_uses_framed_owned_process() {
    let (candidate_root, candidate) = frozen_fixture();
    let (runtime, mut generation) = launch(&candidate, FixtureMode::Normal);
    let snapshot = generation.snapshot().unwrap();
    assert!(!snapshot.pids.is_empty());
    assert!(snapshot.rss_bytes > 0);
    let request_id = generation.begin_probe().unwrap();
    let result = generation.finish_probe(request_id).unwrap();
    assert_eq!(result.output_bytes, 14);
    assert_eq!(result.output_sha256, digest("fixture-output"));
    assert_eq!(result.chunk_count, 1);
    let shutdown = generation.shutdown().unwrap();
    assert_eq!(shutdown.surviving_processes, 0);
    remove(runtime);
    remove(candidate_root);
}

#[test]
fn maximum_valid_chunk_burst_is_not_rejected_by_queue_timing() {
    let (candidate_root, candidate) = frozen_fixture();
    let (runtime, mut generation) = launch(&candidate, FixtureMode::ValidBurst);
    let request_id = generation.begin_probe().unwrap();
    let result = generation.finish_probe(request_id).unwrap();
    assert_eq!(result.chunk_count, MAX_CHUNKS);
    assert_eq!(result.output_bytes, MAX_CHUNKS as u64);
    assert_eq!(
        result.output_sha256,
        digest(&"x".repeat(MAX_CHUNKS as usize))
    );
    assert_eq!(generation.shutdown().unwrap().surviving_processes, 0);
    remove(runtime);
    remove(candidate_root);
}

#[test]
fn kernel_denies_fixture_network_attempt() {
    let (candidate_root, candidate) = frozen_fixture();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let control = TcpStream::connect(address).unwrap();
    let (accepted, _) = listener.accept().unwrap();
    drop(control);
    drop(accepted);
    let (runtime, mut generation) = launch(&candidate, FixtureMode::NetworkAttempt(address.port()));
    let request_id = generation.begin_probe().unwrap();
    let result = generation.finish_probe(request_id).unwrap();
    assert_eq!(result.output_sha256, digest("network-denied-policy"));
    assert_eq!(generation.shutdown().unwrap().surviving_processes, 0);
    remove(runtime);
    remove(candidate_root);
}

#[test]
fn kernel_denies_fixture_process_fork_by_default() {
    let (candidate_root, candidate) = frozen_fixture();
    assert!(Command::new("/usr/bin/true").status().unwrap().success());
    let (runtime, mut generation) = launch(&candidate, FixtureMode::ForkAttempt);
    let request_id = generation.begin_probe().unwrap();
    let result = generation.finish_probe(request_id).unwrap();
    assert_eq!(result.output_sha256, digest("fork-denied-policy"));
    assert_eq!(generation.shutdown().unwrap().surviving_processes, 0);
    remove(runtime);
    remove(candidate_root);
}

#[test]
fn cancel_is_acknowledged_without_late_frame() {
    let (candidate_root, candidate) = frozen_fixture();
    let (runtime, mut generation) = launch(&candidate, FixtureMode::CancelDelay);
    let request_id = generation.begin_probe().unwrap();
    let receipt = generation.cancel(request_id).unwrap();
    assert!(receipt.acknowledgement_millis <= CANCEL_ACK_TIMEOUT_MS);
    assert_eq!(receipt.late_frames, 0);
    assert_eq!(generation.shutdown().unwrap().surviving_processes, 0);
    remove(runtime);
    remove(candidate_root);
}

#[test]
fn late_output_after_cancel_is_rejected() {
    let (candidate_root, candidate) = frozen_fixture();
    let (runtime, mut generation) = launch(&candidate, FixtureMode::CancelLate);
    let request_id = generation.begin_probe().unwrap();
    let receipt = generation.cancel(request_id).unwrap();
    assert_eq!(receipt.late_frames, 1);
    assert!(matches!(
        generation.shutdown(),
        Err(ProcessError::Ownership)
    ));
    remove(runtime);
    remove(candidate_root);
}

#[test]
fn ignored_cancel_kills_entire_process_group() {
    let (candidate_root, candidate) = frozen_fixture();
    let (runtime, mut generation) = launch(&candidate, FixtureMode::CancelIgnore);
    let process_group = generation.process_group_id();
    let request_id = generation.begin_probe().unwrap();
    assert!(matches!(
        generation.cancel(request_id),
        Err(ProcessError::CancelTimeout)
    ));
    assert!(process_group_snapshot(process_group)
        .unwrap()
        .pids
        .is_empty());
    drop(generation);
    remove(runtime);
    remove(candidate_root);
}

#[test]
fn descendant_cannot_survive_owner_shutdown() {
    let (candidate_root, candidate) = frozen_fixture();
    let (runtime, mut generation) = launch(&candidate, FixtureMode::Descendant);
    let request_id = generation.begin_probe().unwrap();
    generation.finish_probe(request_id).unwrap();
    let process_group = generation.process_group_id();
    assert!(generation.snapshot().unwrap().pids.len() >= 2);
    let receipt = generation.shutdown().unwrap();
    assert!(receipt.forced);
    assert_eq!(receipt.surviving_processes, 0);
    assert!(process_group_snapshot(process_group)
        .unwrap()
        .pids
        .is_empty());
    remove(runtime);
    remove(candidate_root);
}

#[test]
fn malformed_crash_and_stderr_flood_are_bounded() {
    let (candidate_root, candidate) = frozen_fixture();
    for mode in [FixtureMode::Malformed, FixtureMode::Crash] {
        let (runtime, mut generation) = launch(&candidate, mode);
        let request_id = generation.begin_probe().unwrap();
        assert!(generation.finish_probe(request_id).is_err());
        drop(generation);
        remove(runtime);
    }

    let (runtime, mut generation) = launch(&candidate, FixtureMode::StderrFlood);
    let request_id = generation.begin_probe().unwrap();
    generation.finish_probe(request_id).unwrap();
    let receipt = generation.shutdown().unwrap();
    assert_eq!(receipt.stderr.bytes, 1_048_576);
    assert_eq!(receipt.stderr.sha256.len(), 64);
    remove(runtime);
    remove(candidate_root);
}

#[test]
fn fixed_resource_budgets_fail_closed() {
    let (candidate_root, candidate) = frozen_fixture();
    for mode in [
        FixtureMode::OutputFlood,
        FixtureMode::ProcessFlood,
        FixtureMode::ProcessPulse,
        FixtureMode::RssSpike,
        FixtureMode::StderrContinuous,
    ] {
        let (runtime, mut generation) = launch(&candidate, mode);
        let request_id = generation.begin_probe().unwrap();
        let result = generation.finish_probe(request_id);
        assert!(
            matches!(result, Err(ProcessError::ResourceLimit(_))),
            "mode {mode:?} returned {result:?}"
        );
        drop(generation);
        remove(runtime);
    }

    let (runtime, mut generation) = launch(&candidate, FixtureMode::StderrOverflow);
    let request_id = generation.begin_probe().unwrap();
    assert!(matches!(
        generation.finish_probe(request_id),
        Err(ProcessError::ResourceLimit(_))
    ));
    drop(generation);
    remove(runtime);
    remove(candidate_root);
}

#[test]
fn response_flood_cannot_block_shutdown_or_drop() {
    let (candidate_root, candidate) = frozen_fixture();

    let (shutdown_runtime, mut shutdown_generation) =
        launch(&candidate, FixtureMode::ResponseFlood);
    shutdown_generation.begin_probe().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let shutdown_started = Instant::now();
    let _ = shutdown_generation.shutdown();
    assert!(shutdown_started.elapsed() < Duration::from_secs(3));
    remove(shutdown_runtime);

    let (drop_runtime, mut drop_generation) = launch(&candidate, FixtureMode::ResponseFlood);
    drop_generation.begin_probe().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let drop_started = Instant::now();
    drop(drop_generation);
    assert!(drop_started.elapsed() < Duration::from_secs(3));
    remove(drop_runtime);
    remove(candidate_root);
}

#[test]
fn measured_boundary_evidence_replays_and_cleanup_is_verified() {
    let (candidate_root, candidate) = frozen_fixture();
    let evidence_root = private_dir("evidence");
    let workspace_a = private_path("evidence-run-a");
    let run_a = run_fixture_boundary(&candidate, &workspace_a).unwrap();
    assert!(!workspace_a.exists());
    assert!(
        publish_evidence(&evidence_root, &candidate, &run_a)
            .unwrap()
            .accepted
    );

    let workspace_b = private_path("evidence-run-b");
    let run_b = run_fixture_boundary(&candidate, &workspace_b).unwrap();
    assert!(!workspace_b.exists());
    assert!(verify_evidence_dir(&evidence_root, &candidate, &run_b).is_err());
    assert!(
        verify_evidence_dir(&evidence_root, &candidate, &run_a)
            .unwrap()
            .accepted
    );
    remove(evidence_root);
    remove(candidate_root);
}
