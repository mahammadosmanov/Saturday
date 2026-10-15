use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use saturday_voice_types::{AsrError, CancelToken};
use saturday_voice_parent_auth::verify_parent_worker_grant;
use saturday_voice_worker_protocol::{
    read_frame, write_frame, WorkerFrame, WorkerMessage, WorkerRole, WireU64, MAX_ASR_MILLIS,
    PROTOCOL_VERSION,
};

mod asr;

const CONTROL_FD: i32 = 3;
const MAX_MODEL_BYTES: u64 = 4_000_000_000;
const SOURCE_SEAL: Option<&str> = option_env!("SATURDAY_VOICE_SOURCE_SEAL_SHA256");
const RESOURCE_SEAL: Option<&str> = option_env!("SATURDAY_VOICE_RESOURCE_SEAL_SHA256");
const TRUSTED_OWNER_PUBLIC_KEY_X963: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/trusted_owner_public_key.x963"));


#[derive(Clone)]
struct Sender(Arc<Mutex<Writer>>);
struct Writer { stream: UnixStream, sequence: u64, operation_id: u64 }
impl Sender {
    fn send(&self, message: WorkerMessage) -> Result<(), ()> {
        let mut writer = self.0.lock().map_err(|_| ())?;
        writer.sequence = writer.sequence.checked_add(1).ok_or(())?;
        let frame = WorkerFrame {
            protocol_version: PROTOCOL_VERSION,
            sequence: WireU64(writer.sequence),
            operation_id: WireU64(writer.operation_id),
            message,
        };
        write_frame(&mut writer.stream, &frame, &[]).map_err(|_| ())
    }
}

struct Control { stopped: AtomicBool }

fn main() { if run().is_err() { std::process::exit(70); } }
fn accepts_control_args(args: &[std::ffi::OsString]) -> bool {
    args.len() == 2 && args[1] == "--control-fd=3"
}

fn run() -> Result<(), ()> {

    let args: Vec<_> = std::env::args_os().collect();
    if !accepts_control_args(&args) {
        return Err(());
    }
    let source_seal = SOURCE_SEAL.ok_or(())?;
    let resource_seal = RESOURCE_SEAL.ok_or(())?;
    if !lower_sha256(source_seal) || !lower_sha256(resource_seal) {
        return Err(());
    }
    let mut stream = unsafe { UnixStream::from_raw_fd(CONTROL_FD) };
    stream.set_read_timeout(Some(Duration::from_secs(2))).map_err(|_| ())?;
    let mut challenge_bytes = [0u8; 32];
    getrandom::fill(&mut challenge_bytes).map_err(|_| ())?;
    let challenge = hex_lower(&challenge_bytes);
    write_frame(&mut stream, &WorkerFrame {
        protocol_version: PROTOCOL_VERSION,
        sequence: WireU64(1),
        operation_id: WireU64(0),
        message: WorkerMessage::Challenge { role: WorkerRole::Asr, challenge: challenge.clone() },
    }, &[]).map_err(|_| ())?;
    let (hello, rights) = read_frame(&mut stream).map_err(|_| ())?;
    if !rights.is_empty() || hello.sequence.0 != 1 || hello.operation_id.0 != 0 {
        return Err(());
    }
    let (hello_challenge, parent_grant) = match hello.message {
        WorkerMessage::Hello { challenge, parent_grant } => (challenge, parent_grant),
        _ => return Err(()),
    };
    let parent_pid = unsafe { libc::getppid() };
    let worker_sha = hash_current_executable()?;
    if parent_pid <= 1
        || hello_challenge != challenge
        || verify_parent_worker_grant(
            &parent_grant,
            saturday_voice_parent_auth::ParentWorkerGrantContext {
                observed_parent_pid: parent_pid,
                role: saturday_voice_parent_auth::WorkerRole::Asr,
                worker_sha256: &worker_sha,
                source_seal_sha256: source_seal,
                resource_seal_sha256: resource_seal,
                trusted_owner_public_key_x963: TRUSTED_OWNER_PUBLIC_KEY_X963,
                expected_challenge_hex: &challenge,
            },
        ).is_err()
    {
        return Err(());
    }
    stream.set_read_timeout(None).map_err(|_| ())?;
    write_frame(&mut stream, &WorkerFrame {
        protocol_version: PROTOCOL_VERSION,
        sequence: WireU64(2),
        operation_id: WireU64(0),
        message: WorkerMessage::Ready { role: WorkerRole::Asr, challenge, worker_pid: std::process::id(),
            source_seal_sha256: source_seal.to_owned(), resource_seal_sha256: resource_seal.to_owned() },
    }, &[]).map_err(|_| ())?;
    let (start, rights) = read_frame(&mut stream).map_err(|_| ())?;
    if start.sequence.0 != 2 || start.operation_id.0 == 0 || rights.len() != 2 { return Err(()); }
    let (sample_count, duration_capped, language_mode, remaining_millis, model_bytes) = match start.message {
        WorkerMessage::StartAsr { sample_count, duration_capped, language_mode, remaining_millis, model_bytes } =>
            (sample_count, duration_capped, language_mode, remaining_millis, model_bytes.0),
        _ => return Err(()),
    };
    if remaining_millis == 0 || remaining_millis > MAX_ASR_MILLIS || model_bytes == 0 || model_bytes > MAX_MODEL_BYTES { return Err(()); }
    let mut rights = rights.into_iter();
    let model = File::from(rights.next().ok_or(())?);
    let pcm = File::from(rights.next().ok_or(())?);
    validate_model(&model, model_bytes)?;
    validate_pcm(&pcm, sample_count)?;
    let operation_id = start.operation_id.0;
    let sender = Sender(Arc::new(Mutex::new(Writer { stream: stream.try_clone().map_err(|_| ())?, sequence: 2, operation_id })));
    sender.send(WorkerMessage::Started {})?;
    let cancel = CancelToken::new();
    let listener_cancel = cancel.clone();
    let control = Arc::new(Control { stopped: AtomicBool::new(false) });
    let listener_control = control.clone();
    let listener = thread::Builder::new().name("voice-asr-control".into()).spawn(move || {
        let mut control_stream = stream;
        let mut expected_sequence = 3u64;
        let _ = control_stream.set_read_timeout(Some(Duration::from_millis(20)));
        while !listener_control.stopped.load(Ordering::Acquire) {
            let (frame, received_rights) = match read_frame(&mut control_stream) {
                Ok(v) => v,
                Err(saturday_voice_worker_protocol::ProtocolError::Io(error))
                    if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => continue,
                Err(_) => { listener_cancel.cancel(); break; }
            };
            if !received_rights.is_empty() || frame.sequence.0 != expected_sequence || frame.operation_id.0 != operation_id {
                listener_cancel.cancel();
                break;
            }
            expected_sequence = match expected_sequence.checked_add(1) { Some(v) => v, None => { listener_cancel.cancel(); break; } };
            if matches!(frame.message, WorkerMessage::Cancel { .. }) { listener_cancel.cancel(); break; }
            listener_cancel.cancel();
            break;
        }
    }).map_err(|_| ())?;
    let deadline = Instant::now() + Duration::from_millis(u64::from(remaining_millis));
    let result = asr::run_asr_worker(model, pcm, sample_count, language_mode, &cancel, deadline);
    control.stopped.store(true, Ordering::Release);
    let _ = listener.join();
    if cancel.is_cancelled() { sender.send(WorkerMessage::Cancelled {})?; return Ok(()); }
    match result {
        Ok(mut result) => {
            if duration_capped {
                result.eligibility = saturday_voice_types::TranscriptEligibility::NeedsReview;
                if !result.quality_flags.contains(&saturday_voice_types::AsrQualityFlag::Truncated) {
                    result.quality_flags.push(saturday_voice_types::AsrQualityFlag::Truncated);
                }
            }
            let result_json = serde_json::to_string(&result).map_err(|_| ())?;
            sender.send(WorkerMessage::AsrComplete { result_json })?;
        }
        Err(AsrError::Cancelled) => sender.send(WorkerMessage::Cancelled {})?,
        Err(AsrError::Timeout) => sender.send(WorkerMessage::Failure { code: saturday_voice_worker_protocol::WorkerFailure::Timeout })?,
        Err(AsrError::AssetUnavailable) => sender.send(WorkerMessage::Failure { code: saturday_voice_worker_protocol::WorkerFailure::InvalidModel })?,
        Err(AsrError::InvalidAudio | AsrError::InvalidEvidence) => sender.send(WorkerMessage::Failure { code: saturday_voice_worker_protocol::WorkerFailure::InvalidAudio })?,
        Err(_) => sender.send(WorkerMessage::Failure { code: saturday_voice_worker_protocol::WorkerFailure::InferenceFailed })?,
    }
    Ok(())
}

fn validate_model(file: &File, size: u64) -> Result<(), ()> {
    let metadata = file.metadata().map_err(|_| ())?;
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if !metadata.is_file() || metadata.len() != size || flags < 0 || flags & libc::O_ACCMODE != libc::O_RDONLY { return Err(()); }
    Ok(())
}
fn validate_pcm(file: &File, samples: u32) -> Result<(), ()> {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(file.as_raw_fd(), &mut stat) } != 0 || (stat.st_mode & libc::S_IFMT) != libc::S_IFIFO
        || samples == 0 || samples > 480_000 { return Err(()); }
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 || flags & libc::O_ACCMODE != libc::O_RDONLY { return Err(()); }
    Ok(())
}
fn lower_sha256(value: &str) -> bool { value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }
fn hash_current_executable() -> Result<String, ()> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = File::open(std::env::current_exe().map_err(|_| ())?).map_err(|_| ())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| ())?;
        if count == 0 { break; }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs::OpenOptions;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temp_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("saturday-voice-asr-{label}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
    }

    #[test]
    fn accepts_only_fixed_control_descriptor_argument() {
        assert!(accepts_control_args(&[OsString::from("worker"), OsString::from("--control-fd=3")]));
        assert!(!accepts_control_args(&[OsString::from("worker")]));
        assert!(!accepts_control_args(&[OsString::from("worker"), OsString::from("--control-fd=4")]));
        assert!(!accepts_control_args(&[OsString::from("worker"), OsString::from("--control-fd=3"), OsString::from("/model.gguf")]));
    }

    #[test]
    fn model_grant_must_be_exact_read_only_regular_file() {
        let path = temp_path("model");
        std::fs::write(&path, b"abc").unwrap();
        let readonly = File::open(&path).unwrap();
        assert!(validate_model(&readonly, 3).is_ok());
        assert!(validate_model(&readonly, 4).is_err());
        let writable = OpenOptions::new().read(true).write(true).open(&path).unwrap();
        assert!(validate_model(&writable, 3).is_err());
        drop((readonly, writable));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn pcm_grant_must_be_read_only_fifo_with_bounded_sample_count() {
        let path = temp_path("pcm");
        let path_c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path_c.as_ptr(), 0o600) }, 0);
        let readonly = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&path).unwrap();
        assert!(validate_pcm(&readonly, 1).is_ok());
        assert!(validate_pcm(&readonly, 0).is_err());
        assert!(validate_pcm(&readonly, 480_001).is_err());
        drop(readonly);
        std::fs::remove_file(path).unwrap();
    }
}
