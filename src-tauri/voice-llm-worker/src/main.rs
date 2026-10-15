use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use saturday_voice_parent_auth::verify_parent_worker_grant;
use saturday_voice_types::{CancelToken, ConversationFailure};
use saturday_voice_worker_protocol::{
    read_frame, write_frame, WorkerFrame, WorkerMessage, WorkerRole, WireU64, MAX_LLM_MILLIS,
    PROTOCOL_VERSION,
};

mod llm;

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
        write_frame(&mut writer.stream, &WorkerFrame { protocol_version: PROTOCOL_VERSION,
            sequence: WireU64(writer.sequence), operation_id: WireU64(writer.operation_id), message }, &[]).map_err(|_| ())
    }
}
enum Control { Credit(u16), Cancel }
struct ControlState { stopped: AtomicBool }
fn main() { if run().is_err() { std::process::exit(70); } }
fn accepts_control_args(args: &[std::ffi::OsString]) -> bool {
    args.len() == 2 && args[1] == "--control-fd=3"
}

fn run() -> Result<(), ()> {

    let args: Vec<_> = std::env::args_os().collect();
    if !accepts_control_args(&args) { return Err(()); }

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
        message: WorkerMessage::Challenge { role: WorkerRole::Llm, challenge: challenge.clone() },
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
                role: saturday_voice_parent_auth::WorkerRole::Llm,
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
        message: WorkerMessage::Ready { role: WorkerRole::Llm, challenge, worker_pid: std::process::id(),
            source_seal_sha256: source_seal.to_owned(), resource_seal_sha256: resource_seal.to_owned() },
    }, &[]).map_err(|_| ())?;
    let (start, rights) = read_frame(&mut stream).map_err(|_| ())?;
    if start.sequence.0 != 2 || start.operation_id.0 == 0 || rights.len() != 1 { return Err(()); }
    let (prompt, user_text, response_language, template_sha, remaining_millis, max_tokens, model_bytes) = match start.message {
        WorkerMessage::StartLlm { prompt, user_text, response_language, chat_template_sha256, remaining_millis, max_output_tokens, model_bytes } =>
            (prompt, user_text, response_language, chat_template_sha256, remaining_millis, max_output_tokens, model_bytes.0),
        _ => return Err(()),
    };
    if remaining_millis == 0 || remaining_millis > MAX_LLM_MILLIS || max_tokens == 0 || max_tokens > 256
        || model_bytes == 0 || model_bytes > MAX_MODEL_BYTES || !lower_sha256(&template_sha) { return Err(()); }
    let model = File::from(rights.into_iter().next().ok_or(())?);
    validate_model(&model, model_bytes)?;
    let operation_id = start.operation_id.0;
    let sender = Sender(Arc::new(Mutex::new(Writer { stream: stream.try_clone().map_err(|_| ())?, sequence: 2, operation_id })));
    sender.send(WorkerMessage::Started {})?;
    let cancel = CancelToken::new();
    let reader_cancel = cancel.clone();
    let control = Arc::new(ControlState { stopped: AtomicBool::new(false) });
    let listener_control = control.clone();
    let (events_tx, events_rx) = mpsc::sync_channel(1);
    let reader = thread::Builder::new().name("voice-llm-control".into()).spawn(move || {
        let mut control_stream = stream;
        let mut expected_sequence = 3u64;
        let _ = control_stream.set_read_timeout(Some(Duration::from_millis(20)));
        while !listener_control.stopped.load(Ordering::Acquire) {
            let (frame, rights) = match read_frame(&mut control_stream) {
                Ok(v) => v,
                Err(saturday_voice_worker_protocol::ProtocolError::Io(error))
                    if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => continue,
                Err(_) => { reader_cancel.cancel(); break; }
            };
            if !rights.is_empty() || frame.sequence.0 != expected_sequence || frame.operation_id.0 != operation_id {
                reader_cancel.cancel();
                break;
            }
            expected_sequence = match expected_sequence.checked_add(1) { Some(v) => v, None => { reader_cancel.cancel(); break; } };
            match frame.message {
                WorkerMessage::Credit { max_delta_bytes } => {
                    if events_tx.try_send(Control::Credit(max_delta_bytes)).is_err() {
                        reader_cancel.cancel();
                        break;
                    }
                }
                WorkerMessage::Cancel { .. } => { reader_cancel.cancel(); let _ = events_tx.try_send(Control::Cancel); break; }
                _ => { reader_cancel.cancel(); break; }
            }
        }
    }).map_err(|_| ())?;
    let deadline = Instant::now() + Duration::from_millis(u64::from(remaining_millis));
    let mut next_credit = || -> Result<u16, ConversationFailure> {
        loop {
            if cancel.is_cancelled() { return Err(ConversationFailure::Cancelled); }
            let now = Instant::now();
            if now >= deadline { cancel.cancel(); return Err(ConversationFailure::Timeout); }
            match events_rx.recv_timeout((deadline - now).min(Duration::from_millis(20))) {
                Ok(Control::Credit(bytes)) => return Ok(bytes),
                Ok(Control::Cancel) => return Err(ConversationFailure::Cancelled),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ConversationFailure::WorkerProtocol),
            }
        }
    };
    let mut delta_sequence = 0u16;
    let mut on_delta = |text: &str, tokens: u16| -> bool {
        if cancel.is_cancelled() { return false; }
        delta_sequence = match delta_sequence.checked_add(1) { Some(n) => n, None => return false };
        sender.send(WorkerMessage::TextDelta { delta_sequence, delta_tokens: tokens, text: text.to_owned() }).is_ok()
    };
    let result = llm::run(model, &template_sha, &prompt, &user_text, response_language, max_tokens,
        &cancel, deadline, &mut next_credit, &mut on_delta);
    drop(on_delta);
    drop(next_credit);
    control.stopped.store(true, Ordering::Release);
    let _ = reader.join();
    if cancel.is_cancelled() || matches!(result, Err(ConversationFailure::Cancelled)) { sender.send(WorkerMessage::Cancelled {})?; return Ok(()); }
    match result {
        Ok(metadata) => sender.send(WorkerMessage::LlmComplete { output_bytes: metadata.output_bytes,
            output_tokens: metadata.output_tokens, input_tokens: metadata.input_tokens,
            verified_template_sha256: metadata.verified_template_sha256, answer_sha256: metadata.answer_sha256 })?,
        Err(ConversationFailure::Timeout) => sender.send(WorkerMessage::Failure { code: saturday_voice_worker_protocol::WorkerFailure::Timeout })?,
        Err(ConversationFailure::AssetUnavailable | ConversationFailure::Unavailable) => sender.send(WorkerMessage::Failure { code: saturday_voice_worker_protocol::WorkerFailure::RuntimeUnavailable })?,
        Err(ConversationFailure::InvalidContext | ConversationFailure::InvalidOutput) => sender.send(WorkerMessage::Failure { code: saturday_voice_worker_protocol::WorkerFailure::InvalidRequest })?,
        Err(ConversationFailure::Backpressure) => sender.send(WorkerMessage::Failure { code: saturday_voice_worker_protocol::WorkerFailure::ResourceLimit })?,
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
fn lower_sha256(value: &str) -> bool { value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }
fn hash_current_executable() -> Result<String, ()> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = File::open(std::env::current_exe().map_err(|_| ())?).map_err(|_| ())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop { let count = file.read(&mut buffer).map_err(|_| ())?; if count == 0 { break; } hasher.update(&buffer[..count]); }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs::OpenOptions;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn accepts_only_fixed_control_descriptor_argument() {
        assert!(accepts_control_args(&[OsString::from("worker"), OsString::from("--control-fd=3")]));
        assert!(!accepts_control_args(&[OsString::from("worker")]));
        assert!(!accepts_control_args(&[OsString::from("worker"), OsString::from("--control-fd=4")]));
        assert!(!accepts_control_args(&[OsString::from("worker"), OsString::from("--control-fd=3"), OsString::from("/model.gguf")]));
    }

    #[test]
    fn model_grant_must_be_exact_read_only_regular_file() {
        let path = std::env::temp_dir().join(format!("saturday-voice-llm-model-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&path, b"abc").unwrap();
        let readonly = File::open(&path).unwrap();
        assert!(validate_model(&readonly, 3).is_ok());
        assert!(validate_model(&readonly, 4).is_err());
        let writable = OpenOptions::new().read(true).write(true).open(&path).unwrap();
        assert!(validate_model(&writable, 3).is_err());
        drop((readonly, writable));
        std::fs::remove_file(path).unwrap();
    }
}
