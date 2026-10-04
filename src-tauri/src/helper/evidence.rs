use crate::helper::manifest::{
    sha256_bytes, CandidateManifestV1, FrozenCandidate, EVIDENCE_SCHEMA, MAX_CHUNKS,
    MAX_OUTPUT_BYTES, MAX_PEAK_RSS_BYTES, MAX_PROCESS_COUNT, MAX_STDERR_BYTES,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use thiserror::Error;

pub const HARNESS_VERSION: &str = "1";
const MAX_EVIDENCE_FILE_BYTES: u64 = 65_536;
const REQUIRED_FILES: [&str; 4] = [
    "inventory.json",
    "manifest.json",
    "probe-counters.json",
    "verdict.json",
];

struct EvidenceDirectory {
    file: File,
}

struct EvidenceFile {
    bytes: Vec<u8>,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceContext {
    pub schema: String,
    pub harness_version: String,
    pub run_id: String,
    pub manifest_sha256: String,
    pub executable_sha256: String,
    pub protocol_version: u16,
    pub generations: BTreeMap<String, String>,
}

impl EvidenceContext {
    fn from_run(candidate: &FrozenCandidate, run: &CompletedBoundaryRun) -> Self {
        Self {
            schema: EVIDENCE_SCHEMA.to_owned(),
            harness_version: HARNESS_VERSION.to_owned(),
            run_id: candidate.manifest().run_id.clone(),
            manifest_sha256: candidate.manifest_sha256().to_owned(),
            executable_sha256: candidate.manifest().executable_sha256.clone(),
            protocol_version: candidate.manifest().protocol_version,
            generations: run.generations.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProbeCounters {
    handshake_ready: bool,
    probes_started: u32,
    probes_completed: u32,
    network_attempt_denied: bool,
    cancel_acknowledged_ms: Option<u64>,
    late_frames: u32,
    surviving_processes: u32,
    peak_process_group_rss_bytes: u64,
    max_process_count: u32,
    max_output_bytes: u64,
    max_chunks: u32,
    max_stderr_bytes: u64,
    temporary_root_removed: bool,
}

pub(crate) struct BoundaryMeasurements {
    pub(crate) handshake_ready: bool,
    pub(crate) probes_started: u32,
    pub(crate) probes_completed: u32,
    pub(crate) network_attempt_denied: bool,
    pub(crate) cancel_acknowledged_ms: Option<u64>,
    pub(crate) late_frames: u32,
    pub(crate) surviving_processes: u32,
    pub(crate) peak_process_group_rss_bytes: u64,
    pub(crate) max_process_count: u32,
    pub(crate) max_output_bytes: u64,
    pub(crate) max_chunks: u32,
    pub(crate) max_stderr_bytes: u64,
    pub(crate) temporary_root_removed: bool,
}

#[derive(Debug)]
pub struct CompletedBoundaryRun {
    run_id: String,
    generations: BTreeMap<String, String>,
    counters: RawProbeCounters,
}

impl CompletedBoundaryRun {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub(crate) fn from_observations(
        run_id: String,
        generations: BTreeMap<String, String>,
        measurements: BoundaryMeasurements,
    ) -> Self {
        Self {
            run_id,
            generations,
            counters: RawProbeCounters {
                handshake_ready: measurements.handshake_ready,
                probes_started: measurements.probes_started,
                probes_completed: measurements.probes_completed,
                network_attempt_denied: measurements.network_attempt_denied,
                cancel_acknowledged_ms: measurements.cancel_acknowledged_ms,
                late_frames: measurements.late_frames,
                surviving_processes: measurements.surviving_processes,
                peak_process_group_rss_bytes: measurements.peak_process_group_rss_bytes,
                max_process_count: measurements.max_process_count,
                max_output_bytes: measurements.max_output_bytes,
                max_chunks: measurements.max_chunks,
                max_stderr_bytes: measurements.max_stderr_bytes,
                temporary_root_removed: measurements.temporary_root_removed,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceVerdict {
    pub accepted: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDocument {
    context: EvidenceContext,
    candidate: CandidateManifestV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CountersDocument {
    context: EvidenceContext,
    counters: RawProbeCounters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerdictDocument {
    context: EvidenceContext,
    verdict: EvidenceVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryDocument {
    context: EvidenceContext,
    files: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("evidence path aliases candidate data")]
    PathAlias,
    #[error("evidence directory is not private and owner-controlled")]
    InsecureDirectory,
    #[error("unexpected evidence file set")]
    UnexpectedFiles,
    #[error("evidence file is invalid: {0}")]
    InvalidFile(&'static str),
    #[error("evidence identity mismatch")]
    IdentityMismatch,
    #[error("evidence I/O failure: {0}")]
    Io(#[from] io::Error),
    #[error("evidence serialization failed")]
    Serialization,
}

fn evaluate_counters(counters: &RawProbeCounters) -> EvidenceVerdict {
    let mut reasons = Vec::new();
    if !counters.handshake_ready {
        reasons.push("handshake_not_ready".to_owned());
    }
    if counters.probes_started != 3 {
        reasons.push("probe_start_count_invalid".to_owned());
    }
    if counters.probes_completed != 2 {
        reasons.push("probe_not_completed".to_owned());
    }
    if !counters.network_attempt_denied {
        reasons.push("network_not_denied".to_owned());
    }
    if counters
        .cancel_acknowledged_ms
        .is_none_or(|millis| millis > 250)
    {
        reasons.push("cancel_ack_budget_failed".to_owned());
    }
    if counters.late_frames != 0 {
        reasons.push("late_frames_observed".to_owned());
    }
    if counters.surviving_processes != 0 {
        reasons.push("surviving_processes_observed".to_owned());
    }
    if counters.peak_process_group_rss_bytes == 0
        || counters.peak_process_group_rss_bytes > MAX_PEAK_RSS_BYTES
        || counters.max_process_count == 0
        || counters.max_process_count > MAX_PROCESS_COUNT
        || counters.max_output_bytes > MAX_OUTPUT_BYTES
        || counters.max_chunks > MAX_CHUNKS
        || counters.max_stderr_bytes > MAX_STDERR_BYTES
    {
        reasons.push("resource_accounting_missing".to_owned());
    }
    if !counters.temporary_root_removed {
        reasons.push("temporary_root_not_removed".to_owned());
    }
    EvidenceVerdict {
        accepted: reasons.is_empty(),
        reasons,
    }
}

pub fn publish_evidence(
    directory: &Path,
    candidate: &FrozenCandidate,
    run: &CompletedBoundaryRun,
) -> Result<EvidenceVerdict, EvidenceError> {
    if run.run_id != candidate.manifest().run_id || !valid_generations(&run.generations) {
        return Err(EvidenceError::IdentityMismatch);
    }
    let directory = validate_evidence_directory(directory, candidate)?;
    ensure_expected_files_absent(&directory)?;
    let context = EvidenceContext::from_run(candidate, run);
    let verdict = evaluate_counters(&run.counters);
    let manifest_bytes = atomic_create_json(
        &directory,
        "manifest.json",
        &ManifestDocument {
            context: context.clone(),
            candidate: candidate.manifest().clone(),
        },
    )?;
    let counters_bytes = atomic_create_json(
        &directory,
        "probe-counters.json",
        &CountersDocument {
            context: context.clone(),
            counters: run.counters.clone(),
        },
    )?;
    let verdict_bytes = atomic_create_json(
        &directory,
        "verdict.json",
        &VerdictDocument {
            context: context.clone(),
            verdict: verdict.clone(),
        },
    )?;
    let mut files = BTreeMap::new();
    files.insert("manifest.json".to_owned(), sha256_bytes(&manifest_bytes));
    files.insert(
        "probe-counters.json".to_owned(),
        sha256_bytes(&counters_bytes),
    );
    files.insert("verdict.json".to_owned(), sha256_bytes(&verdict_bytes));
    atomic_create_json(
        &directory,
        "inventory.json",
        &InventoryDocument { context, files },
    )?;
    Ok(verdict)
}

pub fn verify_evidence_dir(
    directory: &Path,
    candidate: &FrozenCandidate,
    run: &CompletedBoundaryRun,
) -> Result<EvidenceVerdict, EvidenceError> {
    if run.run_id != candidate.manifest().run_id || !valid_generations(&run.generations) {
        return Err(EvidenceError::IdentityMismatch);
    }
    let directory = validate_evidence_directory(directory, candidate)?;
    verify_exact_file_set(&directory)?;
    let expected_context = EvidenceContext::from_run(candidate, run);
    let manifest_file = read_evidence_file(&directory, "manifest.json")?;
    let counters_file = read_evidence_file(&directory, "probe-counters.json")?;
    let verdict_file = read_evidence_file(&directory, "verdict.json")?;
    let inventory_file = read_evidence_file(&directory, "inventory.json")?;
    let manifest: ManifestDocument = parse_strict_json(&manifest_file.bytes)?;
    let counters: CountersDocument = parse_strict_json(&counters_file.bytes)?;
    let verdict: VerdictDocument = parse_strict_json(&verdict_file.bytes)?;
    let inventory: InventoryDocument = parse_strict_json(&inventory_file.bytes)?;
    if manifest.context != expected_context
        || counters.context != expected_context
        || verdict.context != expected_context
        || inventory.context != expected_context
        || manifest.candidate != *candidate.manifest()
        || counters.counters != run.counters
    {
        return Err(EvidenceError::IdentityMismatch);
    }
    let expected_inventory = BTreeMap::from([
        ("manifest.json".to_owned(), manifest_file.sha256),
        ("probe-counters.json".to_owned(), counters_file.sha256),
        ("verdict.json".to_owned(), verdict_file.sha256),
    ]);
    if inventory.files != expected_inventory {
        return Err(EvidenceError::IdentityMismatch);
    }
    let recomputed = evaluate_counters(&counters.counters);
    if verdict.verdict != recomputed {
        return Err(EvidenceError::IdentityMismatch);
    }
    verify_exact_file_set(&directory)?;
    Ok(recomputed)
}

fn validate_evidence_directory(
    directory: &Path,
    candidate: &FrozenCandidate,
) -> Result<EvidenceDirectory, EvidenceError> {
    let canonical = directory.canonicalize()?;
    if canonical.starts_with(candidate.root()) || candidate.root().starts_with(&canonical) {
        return Err(EvidenceError::PathAlias);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&canonical)?;
    let held = file.metadata()?;
    let named = fs::metadata(&canonical)?;
    if !held.is_dir()
        || held.uid() != effective_uid()
        || held.mode() & 0o077 != 0
        || held.dev() != named.dev()
        || held.ino() != named.ino()
    {
        return Err(EvidenceError::InsecureDirectory);
    }
    Ok(EvidenceDirectory { file })
}

fn ensure_expected_files_absent(directory: &EvidenceDirectory) -> Result<(), EvidenceError> {
    if !directory_entries(directory.file.as_raw_fd())?.is_empty() {
        return Err(EvidenceError::UnexpectedFiles);
    }
    Ok(())
}

fn verify_exact_file_set(directory: &EvidenceDirectory) -> Result<(), EvidenceError> {
    let actual = directory_entries(directory.file.as_raw_fd())?;
    let expected = REQUIRED_FILES.into_iter().map(str::to_owned).collect();
    if actual != expected {
        return Err(EvidenceError::UnexpectedFiles);
    }
    Ok(())
}

fn read_evidence_file(
    directory: &EvidenceDirectory,
    name: &str,
) -> Result<EvidenceFile, EvidenceError> {
    require_evidence_name(name)?;
    let name = CString::new(name).map_err(|_| EvidenceError::InvalidFile("filename"))?;
    // SAFETY: directory and name are valid descriptors/strings; the returned descriptor is owned.
    let descriptor = unsafe {
        libc::openat(
            directory.file.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(EvidenceError::Io(io::Error::last_os_error()));
    }
    // SAFETY: `openat` returned a new owned descriptor.
    let mut file = unsafe { File::from_raw_fd(descriptor) };
    validate_evidence_file_metadata(&file.metadata()?)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_EVIDENCE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_EVIDENCE_FILE_BYTES {
        return Err(EvidenceError::InvalidFile("file too large"));
    }
    Ok(EvidenceFile {
        sha256: sha256_bytes(&bytes),
        bytes,
    })
}

fn parse_strict_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, EvidenceError> {
    serde_json::from_slice(bytes).map_err(|_| EvidenceError::InvalidFile("JSON schema"))
}

fn atomic_create_json<T: Serialize>(
    directory: &EvidenceDirectory,
    name: &str,
    value: &T,
) -> Result<Vec<u8>, EvidenceError> {
    require_evidence_name(name)?;
    let mut bytes = serde_json::to_vec(value).map_err(|_| EvidenceError::Serialization)?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_EVIDENCE_FILE_BYTES {
        return Err(EvidenceError::InvalidFile("file too large"));
    }
    let name = CString::new(name).map_err(|_| EvidenceError::InvalidFile("filename"))?;
    // SAFETY: directory and name are valid; O_EXCL prevents replacing an existing entry.
    let descriptor = unsafe {
        libc::openat(
            directory.file.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(EvidenceError::Io(io::Error::last_os_error()));
    }
    // SAFETY: `openat` returned a new owned descriptor.
    let mut file = unsafe { File::from_raw_fd(descriptor) };
    let write_result = (|| -> io::Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        directory.file.sync_all()
    })();
    if let Err(error) = write_result {
        // SAFETY: name belongs to this held directory; cleanup failure keeps verification closed.
        unsafe { libc::unlinkat(directory.file.as_raw_fd(), name.as_ptr(), 0) };
        return Err(EvidenceError::Io(error));
    }
    validate_evidence_file_metadata(&file.metadata()?)?;
    Ok(bytes)
}

fn require_evidence_name(name: &str) -> Result<(), EvidenceError> {
    if REQUIRED_FILES.contains(&name) {
        Ok(())
    } else {
        Err(EvidenceError::InvalidFile("filename"))
    }
}

fn valid_generations(generations: &BTreeMap<String, String>) -> bool {
    let expected = ["cancel", "network", "normal"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let actual = generations.keys().cloned().collect::<BTreeSet<_>>();
    let values = generations.values().collect::<BTreeSet<_>>();
    actual == expected
        && values.len() == generations.len()
        && generations.values().all(|value| {
            value.len() == 32
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn validate_evidence_file_metadata(metadata: &fs::Metadata) -> Result<(), EvidenceError> {
    if !metadata.is_file()
        || metadata.len() > MAX_EVIDENCE_FILE_BYTES
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        Err(EvidenceError::InvalidFile("file metadata"))
    } else {
        Ok(())
    }
}

fn directory_entries(directory_fd: RawFd) -> Result<BTreeSet<String>, EvidenceError> {
    // SAFETY: dup returns an independently owned descriptor on success.
    let duplicate = unsafe { libc::dup(directory_fd) };
    if duplicate < 0 {
        return Err(EvidenceError::Io(io::Error::last_os_error()));
    }
    // SAFETY: fdopendir takes ownership of the duplicated descriptor on success.
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        // SAFETY: fdopendir failed and did not take ownership.
        unsafe { libc::close(duplicate) };
        return Err(EvidenceError::Io(io::Error::last_os_error()));
    }
    // SAFETY: stream is a live directory stream and may be rewound before iteration.
    unsafe { libc::rewinddir(stream) };
    let result = (|| {
        let mut names = BTreeSet::new();
        loop {
            // SAFETY: macOS exposes thread-local errno through __error.
            unsafe { *libc::__error() = 0 };
            // SAFETY: stream remains valid until closed below.
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                // SAFETY: errno storage is valid for this thread.
                let code = unsafe { *libc::__error() };
                if code != 0 {
                    return Err(EvidenceError::Io(io::Error::from_raw_os_error(code)));
                }
                break;
            }
            // SAFETY: readdir returns a null-terminated d_name for the current entry.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()
                .map_err(|_| EvidenceError::UnexpectedFiles)?;
            if name != "." && name != ".." {
                names.insert(name.to_owned());
            }
        }
        Ok(names)
    })();
    // SAFETY: stream was created by fdopendir and must be closed exactly once.
    let close_result = unsafe { libc::closedir(stream) };
    if close_result != 0 && result.is_ok() {
        return Err(EvidenceError::Io(io::Error::last_os_error()));
    }
    result
}

fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no arguments and no memory-safety preconditions.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helper::manifest::{
        freeze_candidate, generate_run_id, sha256_file, CandidateManifestV1, LAUNCHER_POLICY_ID,
        MANIFEST_SCHEMA, MAX_CHUNKS, MAX_OUTPUT_BYTES, MAX_PEAK_RSS_BYTES, MAX_PROCESS_COUNT,
        MAX_STDERR_BYTES, PROBE_TIMEOUT_MS, STARTUP_TIMEOUT_MS,
    };
    use crate::helper::protocol::PROTOCOL_VERSION;
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::{Path, PathBuf};

    fn private_dir(label: &str) -> PathBuf {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).unwrap();
        let suffix = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = std::env::temp_dir().join(format!("jarvis-{label}-{suffix}"));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn remove_private(path: &Path) {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    fn frozen(root: &Path) -> crate::helper::manifest::FrozenCandidate {
        let executable = root.join("fixture-helper");
        fs::write(&executable, b"fixture-bytes").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o500)).unwrap();
        let manifest = CandidateManifestV1 {
            schema: MANIFEST_SCHEMA.to_owned(),
            protocol_version: PROTOCOL_VERSION,
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
            evidence_schema: EVIDENCE_SCHEMA.to_owned(),
        };
        freeze_candidate(root, manifest).unwrap()
    }

    fn passing_counters() -> RawProbeCounters {
        RawProbeCounters {
            handshake_ready: true,
            probes_started: 3,
            probes_completed: 2,
            network_attempt_denied: true,
            cancel_acknowledged_ms: Some(25),
            late_frames: 0,
            surviving_processes: 0,
            peak_process_group_rss_bytes: 1_048_576,
            max_process_count: 1,
            max_output_bytes: 14,
            max_chunks: 1,
            max_stderr_bytes: 0,
            temporary_root_removed: true,
        }
    }

    fn passing_run(candidate: &FrozenCandidate) -> CompletedBoundaryRun {
        let generations = ["normal", "network", "cancel"]
            .into_iter()
            .map(|scenario| (scenario.to_owned(), generate_run_id().unwrap()))
            .collect();
        CompletedBoundaryRun {
            run_id: candidate.manifest().run_id.clone(),
            generations,
            counters: passing_counters(),
        }
    }

    #[test]
    fn verdict_is_recomputed_from_fixed_gates() {
        assert!(evaluate_counters(&passing_counters()).accepted);
        let mut failed = passing_counters();
        failed.probes_completed = 0;
        let verdict = evaluate_counters(&failed);
        assert!(!verdict.accepted);
        assert!(verdict
            .reasons
            .iter()
            .any(|reason| reason == "probe_not_completed"));
    }

    #[test]
    fn publishes_and_replays_manifest_bound_redacted_evidence() {
        let candidate_root = private_dir("candidate");
        let evidence_root = private_dir("evidence");
        let candidate = frozen(&candidate_root);
        let run = passing_run(&candidate);
        publish_evidence(&evidence_root, &candidate, &run).unwrap();
        let replay = verify_evidence_dir(&evidence_root, &candidate, &run).unwrap();
        assert!(replay.accepted);
        let all = fs::read_dir(&evidence_root)
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<String>();
        for forbidden_key in ["stdout", "stderr", "completion", "prompt"] {
            assert!(!all.contains(&format!("\"{forbidden_key}\"")));
        }
        assert!(!all.contains(candidate.executable().to_str().unwrap()));
        remove_private(&candidate_root);
        fs::remove_dir_all(evidence_root).unwrap();
    }

    #[test]
    fn rejects_symlinked_destination_and_unexpected_files() {
        let candidate_root = private_dir("candidate-symlink");
        let evidence_root = private_dir("evidence-symlink");
        let victim = evidence_root.join("victim");
        fs::write(&victim, b"unchanged").unwrap();
        symlink(&victim, evidence_root.join("manifest.json")).unwrap();
        let candidate = frozen(&candidate_root);
        let run = passing_run(&candidate);
        assert!(publish_evidence(&evidence_root, &candidate, &run).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"unchanged");
        fs::remove_file(evidence_root.join("manifest.json")).unwrap();
        fs::remove_file(&victim).unwrap();
        publish_evidence(&evidence_root, &candidate, &run).unwrap();
        fs::write(evidence_root.join("extra.json"), b"{}").unwrap();
        assert!(verify_evidence_dir(&evidence_root, &candidate, &run).is_err());
        remove_private(&candidate_root);
        fs::remove_dir_all(evidence_root).unwrap();
    }

    #[test]
    fn refuses_to_publish_into_a_dirty_directory() {
        let candidate_root = private_dir("dirty-publication-candidate");
        let candidate = frozen(&candidate_root);
        let run = passing_run(&candidate);
        let evidence_root = private_dir("dirty-publication");
        fs::write(evidence_root.join("extra.json"), b"{}\n").unwrap();
        assert!(matches!(
            publish_evidence(&evidence_root, &candidate, &run),
            Err(EvidenceError::UnexpectedFiles)
        ));
        remove_private(&candidate_root);
        fs::remove_dir_all(evidence_root).unwrap();
    }

    #[test]
    fn rejects_candidate_evidence_path_alias() {
        let candidate_root = private_dir("alias");
        let candidate = frozen(&candidate_root);
        let run = passing_run(&candidate);
        assert!(matches!(
            publish_evidence(&candidate_root, &candidate, &run),
            Err(EvidenceError::PathAlias)
        ));
        remove_private(&candidate_root);
    }

    #[test]
    fn rejects_world_readable_and_hardlinked_evidence_files() {
        let candidate_root = private_dir("evidence-metadata-candidate");
        let candidate = frozen(&candidate_root);
        let run = passing_run(&candidate);

        let readable_root = private_dir("evidence-world-readable");
        publish_evidence(&readable_root, &candidate, &run).unwrap();
        fs::set_permissions(
            readable_root.join("manifest.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(verify_evidence_dir(&readable_root, &candidate, &run).is_err());

        let linked_root = private_dir("evidence-hardlink");
        publish_evidence(&linked_root, &candidate, &run).unwrap();
        let alias_root = private_dir("evidence-hardlink-alias");
        fs::hard_link(
            linked_root.join("manifest.json"),
            alias_root.join("manifest-alias.json"),
        )
        .unwrap();
        assert!(verify_evidence_dir(&linked_root, &candidate, &run).is_err());

        remove_private(&candidate_root);
        fs::remove_dir_all(readable_root).unwrap();
        fs::remove_dir_all(linked_root).unwrap();
        fs::remove_dir_all(alias_root).unwrap();
    }

    #[test]
    fn descriptor_reads_survive_directory_swap_and_bound_growth() {
        let candidate_root = private_dir("evidence-descriptor-candidate");
        let candidate = frozen(&candidate_root);
        let run = passing_run(&candidate);
        let evidence_root = private_dir("evidence-descriptor");
        publish_evidence(&evidence_root, &candidate, &run).unwrap();

        let held = validate_evidence_directory(&evidence_root, &candidate).unwrap();
        let moved_root = evidence_root.with_extension("moved");
        fs::rename(&evidence_root, &moved_root).unwrap();
        fs::create_dir(&evidence_root).unwrap();
        fs::set_permissions(&evidence_root, fs::Permissions::from_mode(0o700)).unwrap();
        let manifest = read_evidence_file(&held, "manifest.json").unwrap();
        let parsed: ManifestDocument = parse_strict_json(&manifest.bytes).unwrap();
        assert_eq!(parsed.candidate.run_id, candidate.manifest().run_id);

        let inventory = moved_root.join("inventory.json");
        fs::set_permissions(&inventory, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&inventory, vec![b'x'; MAX_EVIDENCE_FILE_BYTES as usize + 1]).unwrap();
        let moved = validate_evidence_directory(&moved_root, &candidate).unwrap();
        assert!(matches!(
            read_evidence_file(&moved, "inventory.json"),
            Err(EvidenceError::InvalidFile(
                "file metadata" | "file too large"
            ))
        ));

        remove_private(&candidate_root);
        fs::remove_dir_all(evidence_root).unwrap();
        fs::remove_dir_all(moved_root).unwrap();
    }
}
