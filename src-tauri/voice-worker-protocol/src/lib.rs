use saturday_voice_types::{Language, LanguageMode};
use serde::{de::Visitor, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use thiserror::Error;

pub const PROTOCOL_VERSION: u16 = 2;
pub const MAX_FRAME_BYTES: usize = 65_536;
pub const MAX_DELTA_BYTES: usize = 4_096;
pub const MAX_CAPTURE_SAMPLES: u32 = 480_000;
pub const MAX_ASR_MILLIS: u32 = 15_000;
pub const MAX_LLM_MILLIS: u32 = 60_000;
pub const MAX_OUTPUT_TOKENS: u16 = 256;
pub const MAX_TEXT_BYTES: usize = 8_192;
pub const MAX_TYPED_SCALARS: usize = 500;
pub const MAX_USER_TEXT_SCALARS: usize = 2_000;
pub const FD_SENTINEL: u8 = 0xf0;
pub const MAX_PARENT_GRANT_BYTES: usize = 8_192;
pub const MAX_RIGHTS: usize = 2;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("worker protocol I/O failed")]
    Io(#[from] io::Error),
    #[error("worker frame length is invalid")]
    Length,
    #[error("worker frame is malformed")]
    Malformed,
    #[error("worker frame violates protocol bounds")]
    Invalid,
    #[error("worker file descriptor transfer is invalid")]
    Rights,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireU64(pub u64);

impl Serialize for WireU64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for WireU64 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DecimalVisitor;
        impl Visitor<'_> for DecimalVisitor {
            type Value = WireU64;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a canonical unsigned 64-bit decimal string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if value.is_empty()
                    || (value.len() > 1 && value.starts_with('0'))
                    || !value.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return Err(E::custom("non-canonical unsigned integer"));
                }
                value
                    .parse::<u64>()
                    .map(WireU64)
                    .map_err(|_| E::custom("unsigned integer overflow"))
            }
        }
        deserializer.deserialize_str(DecimalVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRole {
    Asr,
    Llm,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    User,
    WindowLost,
    DeviceLost,
    Shutdown,
    Deadline,
    ResourceLimit,
    AuthorizationChanged,
    ProtocolFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerFailure {
    InvalidRequest,
    RuntimeUnavailable,
    InvalidModel,
    InvalidAudio,
    UnsupportedLanguage,
    Cancelled,
    Timeout,
    ResourceLimit,
    InferenceFailed,
    CleanupFailed,
    ProtocolFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerFrame {
    pub protocol_version: u16,
    pub sequence: WireU64,
    pub operation_id: WireU64,
    pub message: WorkerMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerMessage {
    Challenge {
        role: WorkerRole,
        challenge: String,
    },
    Hello {
        challenge: String,
        parent_grant: Vec<u8>,
    },
    Ready {
        role: WorkerRole,
        challenge: String,
        worker_pid: u32,
        source_seal_sha256: String,
        resource_seal_sha256: String,
    },
    StartAsr {
        sample_count: u32,
        duration_capped: bool,
        language_mode: LanguageMode,
        remaining_millis: u32,
        model_bytes: WireU64,
    },
    StartLlm {
        prompt: String,
        user_text: String,
        response_language: Language,
        chat_template_sha256: String,
        remaining_millis: u32,
        max_output_tokens: u16,
        model_bytes: WireU64,
    },
    Started {},
    TextDelta {
        delta_sequence: u16,
        delta_tokens: u16,
        text: String,
    },
    AsrComplete {
        result_json: String,
    },
    LlmComplete {
        output_bytes: u16,
        output_tokens: u16,
        input_tokens: u16,
        verified_template_sha256: String,
        answer_sha256: String,
    },
    Credit {
        max_delta_bytes: u16,
    },
    Cancel {
        reason: CancelReason,
    },
    Cancelled {},
    Failure {
        code: WorkerFailure,
    },
    Shutdown {},
    Bye {},
}

impl WorkerMessage {
    pub fn required_rights(&self) -> usize {
        match self {
            Self::StartAsr { .. } => 2,
            Self::StartLlm { .. } => 1,
            _ => 0,
        }
    }

    pub fn expected_role(&self) -> Option<WorkerRole> {
        match self {
            Self::StartAsr { .. } => Some(WorkerRole::Asr),
            Self::StartLlm { .. } => Some(WorkerRole::Llm),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::Challenge { challenge, .. } => {
                if !is_lower_hex(challenge, 64) {
                    return Err(ProtocolError::Invalid);
                }
            }
            Self::Hello { challenge, parent_grant } => {
                if !is_lower_hex(challenge, 64)
                    || parent_grant.is_empty()
                    || parent_grant.len() > MAX_PARENT_GRANT_BYTES
                {
                    return Err(ProtocolError::Invalid);
                }
            }
            Self::Ready {
                challenge,
                worker_pid,
                source_seal_sha256,
                resource_seal_sha256,
                ..
            } => {
                if *worker_pid == 0
                    || !is_lower_hex(challenge, 64)
                    || !is_lower_hex(source_seal_sha256, 64)
                    || !is_lower_hex(resource_seal_sha256, 64)
                {
                    return Err(ProtocolError::Invalid);
                }
            }
            Self::StartAsr {
                sample_count,
                remaining_millis,
                model_bytes,
                ..
            } => {
                if *sample_count == 0
                    || *sample_count > MAX_CAPTURE_SAMPLES
                    || *remaining_millis == 0
                    || *remaining_millis > MAX_ASR_MILLIS
                    || model_bytes.0 == 0
                {
                    return Err(ProtocolError::Invalid);
                }
            }
            Self::StartLlm {
                prompt,
                user_text,
                chat_template_sha256,
                remaining_millis,
                max_output_tokens,
                model_bytes,
                ..
            } => {
                if prompt.len() > 65_536
                    || user_text.is_empty()
                    || user_text.len() > MAX_TEXT_BYTES
                    || user_text.chars().count() > MAX_USER_TEXT_SCALARS
                    || !is_lower_hex(chat_template_sha256, 64)
                    || *remaining_millis == 0
                    || *remaining_millis > MAX_LLM_MILLIS
                    || *max_output_tokens == 0
                    || *max_output_tokens > MAX_OUTPUT_TOKENS
                    || model_bytes.0 == 0
                {
                    return Err(ProtocolError::Invalid);
                }
            }
            Self::TextDelta { delta_tokens, text, .. }
                if text.is_empty()
                    || text.len() > MAX_DELTA_BYTES
                    || *delta_tokens == 0
                    || *delta_tokens > MAX_OUTPUT_TOKENS =>
            {
                return Err(ProtocolError::Invalid);
            }
            Self::AsrComplete { result_json }
                if result_json.is_empty() || result_json.len() > 20_000 =>
            {
                return Err(ProtocolError::Invalid);
            }
            Self::LlmComplete {
                output_bytes,
                output_tokens,
                input_tokens,
                verified_template_sha256,
                answer_sha256,
            } if *output_bytes == 0
                || usize::from(*output_bytes) > MAX_TEXT_BYTES
                || *output_tokens == 0
                || *output_tokens > MAX_OUTPUT_TOKENS
                || *input_tokens == 0
                || *input_tokens > 4_096
                || !is_lower_hex(verified_template_sha256, 64)
                || !is_lower_hex(answer_sha256, 64) =>
            {
                return Err(ProtocolError::Invalid);
            }
            Self::Credit { max_delta_bytes }
                if *max_delta_bytes == 0 || *max_delta_bytes as usize > MAX_DELTA_BYTES =>
            {
                return Err(ProtocolError::Invalid);
            }
            _ => {}
        }
        Ok(())
    }
}

impl WorkerFrame {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.protocol_version != PROTOCOL_VERSION || self.sequence.0 == 0 {
            return Err(ProtocolError::Invalid);
        }
        self.message.validate()
    }
}

pub fn encode_frame(frame: &WorkerFrame) -> Result<Vec<u8>, ProtocolError> {
    frame.validate()?;
    let json = serde_json::to_vec(frame).map_err(|_| ProtocolError::Malformed)?;
    if json.is_empty() || json.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::Length);
    }
    let len = u32::try_from(json.len()).map_err(|_| ProtocolError::Length)?;
    let mut bytes = Vec::with_capacity(4 + json.len());
    bytes.extend_from_slice(&len.to_be_bytes());
    bytes.extend_from_slice(&json);
    Ok(bytes)
}

pub fn write_frame(
    stream: &mut UnixStream,
    frame: &WorkerFrame,
    rights: &[RawFd],
) -> Result<(), ProtocolError> {
    if rights.len() != frame.message.required_rights() || rights.len() > MAX_RIGHTS {
        return Err(ProtocolError::Rights);
    }
    let encoded = encode_frame(frame)?;
    if !rights.is_empty() {
        send_rights(stream, rights)?;
    }
    stream.write_all(&encoded)?;
    Ok(())
}

pub fn read_frame(stream: &mut UnixStream) -> Result<(WorkerFrame, Vec<OwnedFd>), ProtocolError> {
    let rights = receive_optional_rights(stream)?;
    let mut header = [0u8; 4];
    read_exact_or_eof(stream, &mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(ProtocolError::Length);
    }
    let mut json = vec![0; length];
    stream.read_exact(&mut json)?;
    let frame: WorkerFrame = serde_json::from_slice(&json).map_err(|_| ProtocolError::Malformed)?;
    frame.validate()?;
    if rights.len() != frame.message.required_rights() {
        return Err(ProtocolError::Rights);
    }
    Ok((frame, rights))
}
fn read_exact_or_eof(stream: &mut UnixStream, bytes: &mut [u8]) -> Result<(), ProtocolError> {
    match stream.read_exact(bytes) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            Err(ProtocolError::Io(error))
        }
        Err(error) => Err(ProtocolError::Io(error)),
    }
}

fn send_rights(stream: &UnixStream, rights: &[RawFd]) -> Result<(), ProtocolError> {
    if rights.is_empty() || rights.len() > MAX_RIGHTS {
        return Err(ProtocolError::Rights);
    }
    let rights_len = std::mem::size_of_val(rights);
    let mut byte = [FD_SENTINEL];
    let mut iovec = libc::iovec {
        iov_base: byte.as_mut_ptr().cast(),
        iov_len: byte.len(),
    };
    let control_len = unsafe { libc::CMSG_SPACE(rights_len as _) } as usize;
    let mut control = vec![0u8; control_len];
    // SAFETY: zeroed msghdr is valid before its pointer/length fields are set below.
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iovec;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len() as _;
    // SAFETY: the control buffer has CMSG_SPACE capacity for every descriptor and the
    // iovec references the one-byte sentinel for the duration of sendmsg.
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&message);
        if header.is_null() {
            return Err(ProtocolError::Rights);
        }
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        let cmsg_len = libc::CMSG_LEN(rights_len as _);
        (*header).cmsg_len = cmsg_len;
        std::ptr::copy_nonoverlapping(
            rights.as_ptr().cast::<u8>(),
            libc::CMSG_DATA(header),
            rights_len,
        );
        message.msg_controllen = (*header).cmsg_len as _;
        let sent = libc::sendmsg(stream.as_raw_fd(), &message, 0);
        if sent != 1 {
            return Err(ProtocolError::Io(io::Error::last_os_error()));
        }
    }
    Ok(())
}

fn receive_optional_rights(stream: &UnixStream) -> Result<Vec<OwnedFd>, ProtocolError> {
    let mut peek = [0u8; 1];
    let peeked = unsafe { libc::recv(stream.as_raw_fd(), peek.as_mut_ptr().cast(), 1, libc::MSG_PEEK) };
    if peeked != 1 {
        return Err(ProtocolError::Io(io::Error::last_os_error()));
    }
    if peek[0] != FD_SENTINEL {
        return Ok(Vec::new());
    }
    let mut sentinel = [0u8; 1];
    let mut iovec = libc::iovec {
        iov_base: sentinel.as_mut_ptr().cast(),
        iov_len: sentinel.len(),
    };
    let control_len =
        unsafe { libc::CMSG_SPACE((MAX_RIGHTS * std::mem::size_of::<RawFd>()) as _) } as usize;
    let mut control = vec![0u8; control_len];
    // SAFETY: zeroed msghdr is valid before its pointer/length fields are set below.
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iovec;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len().try_into().map_err(|_| ProtocolError::Rights)?;
    // SAFETY: message buffers remain live for recvmsg; received descriptor payload is
    // copied out only after control-header validation.
    let received = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, 0) };
    let mut result = Vec::new();
    let mut valid = received == 1
        && sentinel[0] == FD_SENTINEL
        && message.msg_flags & libc::MSG_CTRUNC == 0;
    if received >= 0 {
        unsafe {
            let mut header = libc::CMSG_FIRSTHDR(&message);
            while !header.is_null() {
                if (*header).cmsg_level != libc::SOL_SOCKET
                    || (*header).cmsg_type != libc::SCM_RIGHTS
                {
                    valid = false;
                    header = libc::CMSG_NXTHDR(&message, header);
                    continue;
                }
                let Ok(header_len) = usize::try_from((*header).cmsg_len) else {
                    valid = false;
                    break;
                };
                let data_offset = libc::CMSG_LEN(0) as usize;
                let header_offset = header.cast::<u8>().offset_from(control.as_ptr()) as usize;
                let control_len =
                    usize::try_from(message.msg_controllen).map_err(|_| ProtocolError::Rights)?;
                let Some(available_len) = control_len.checked_sub(header_offset) else {
                    valid = false;
                    break;
                };
                if header_len > available_len {
                    valid = false;
                }
                let bounded_len = header_len.min(available_len);
                let Some(payload_bytes) = bounded_len.checked_sub(data_offset) else {
                    valid = false;
                    break;
                };
                if payload_bytes % std::mem::size_of::<RawFd>() != 0 {
                    valid = false;
                }
                let count = payload_bytes / std::mem::size_of::<RawFd>();
                if count == 0 || result.len().saturating_add(count) > MAX_RIGHTS {
                    valid = false;
                }
                let source = libc::CMSG_DATA(header).cast::<RawFd>();
                for index in 0..count {
                    let raw = *source.add(index);
                    if raw < 0 {
                        valid = false;
                    } else {
                        result.push(OwnedFd::from_raw_fd(raw));
                    }
                }
                header = libc::CMSG_NXTHDR(&message, header);
            }
        }
    }
    if !valid || result.is_empty() {
        return Err(ProtocolError::Rights);
    }
    for fd in &result {
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
        if flags < 0 || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(ProtocolError::Rights);
        }
    }
    Ok(result)
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_wire_integer_is_canonical_and_bounded() {
        assert_eq!(serde_json::from_str::<WireU64>("\"0\"").unwrap(), WireU64(0));
        for invalid in ["\"00\"", "\"-1\"", "\"18446744073709551616\"", "1"] {
            assert!(serde_json::from_str::<WireU64>(invalid).is_err());
        }
    }

    #[test]
    fn protocol_rejects_unknown_duplicate_and_oversized_frames() {
        let valid = br#"{"protocol_version":2,"sequence":"1","operation_id":"2","message":{"type":"started"}}"#;
        assert!(serde_json::from_slice::<WorkerFrame>(valid).is_ok());
        for kind in ["started", "cancelled", "shutdown", "bye"] {
            let unknown = format!(
                r#"{{"protocol_version":2,"sequence":"1","operation_id":"2","message":{{"type":"{kind}","extra":true}}}}"#
            );
            assert!(serde_json::from_str::<WorkerFrame>(&unknown).is_err());
        }
        let duplicate = br#"{"protocol_version":2,"protocol_version":2,"sequence":"1","operation_id":"2","message":{"type":"started"}}"#;
        assert!(serde_json::from_slice::<WorkerFrame>(duplicate).is_err());
        assert!(encode_frame(&WorkerFrame {
            protocol_version: PROTOCOL_VERSION,
            sequence: WireU64(1),
            operation_id: WireU64(2),
            message: WorkerMessage::Started {},
        }).is_ok());
    }

    #[test]
    fn worker_challenge_is_bounded_role_scoped_and_strict() {
        let valid = br#"{"protocol_version":2,"sequence":"1","operation_id":"0","message":{"type":"challenge","role":"asr","challenge":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#;
        let frame: WorkerFrame = serde_json::from_slice(valid).unwrap();
        assert!(frame.validate().is_ok());
        assert_eq!(frame.message.required_rights(), 0);

        let malformed = br#"{"protocol_version":2,"sequence":"1","operation_id":"0","message":{"type":"challenge","role":"asr","challenge":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}}"#;
        assert!(serde_json::from_slice::<WorkerFrame>(malformed).unwrap().validate().is_err());

        let caller_claim = br#"{"protocol_version":2,"sequence":"1","operation_id":"0","message":{"type":"challenge","role":"asr","challenge":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","parent_pid":42}}"#;
        assert!(serde_json::from_slice::<WorkerFrame>(caller_claim).is_err());
    }
    #[test]
    fn descriptor_count_is_tied_to_fixed_worker_operation() {
        assert_eq!(WorkerMessage::StartAsr {
            sample_count: 1,
            duration_capped: false,
            language_mode: LanguageMode::Auto,
            remaining_millis: 1,
            model_bytes: WireU64(1),
        }.required_rights(), 2);
        assert_eq!(WorkerMessage::StartLlm {
            prompt: String::new(),
            user_text: "x".into(),
            response_language: Language::En,
            chat_template_sha256: "a".repeat(64),
            remaining_millis: 1,
            max_output_tokens: 1,
            model_bytes: WireU64(1),
        }.required_rights(), 1);
    }

    #[test]
    fn llm_user_text_enforces_unicode_scalar_limit() {
        let message = |scalar_count: usize| WorkerMessage::StartLlm {
            prompt: String::new(),
            user_text: "é".repeat(scalar_count),
            response_language: Language::En,
            chat_template_sha256: "a".repeat(64),
            remaining_millis: 1,
            max_output_tokens: 1,
            model_bytes: WireU64(1),
        };
        assert!(message(MAX_USER_TEXT_SCALARS).validate().is_ok());
        assert!(message(MAX_USER_TEXT_SCALARS + 1).validate().is_err());
    }
    #[test]
    fn fixed_asr_descriptor_transfer_roundtrips_with_close_on_exec() {
        let (mut sender, mut receiver) = UnixStream::pair().unwrap();
        let samples = std::fs::File::open("/dev/null").unwrap();
        let model = std::fs::File::open("/dev/null").unwrap();
        let frame = WorkerFrame {
            protocol_version: PROTOCOL_VERSION,
            sequence: WireU64(1),
            operation_id: WireU64(2),
            message: WorkerMessage::StartAsr {
                sample_count: 1,
                duration_capped: false,
                language_mode: LanguageMode::Auto,
                remaining_millis: 1,
                model_bytes: WireU64(1),
            },
        };

        write_frame(
            &mut sender,
            &frame,
            &[samples.as_raw_fd(), model.as_raw_fd()],
        )
        .unwrap();
        let (received, rights) = read_frame(&mut receiver).unwrap();

        assert_eq!(received, frame);
        assert_eq!(rights.len(), MAX_RIGHTS);
        assert!(rights.iter().all(|fd| unsafe {
            libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) & libc::FD_CLOEXEC != 0
        }));
    }

    #[test]
    fn authorization_change_cancellation_is_a_strict_wire_reason() {
        let frame = WorkerFrame {
            protocol_version: PROTOCOL_VERSION,
            sequence: WireU64(3),
            operation_id: WireU64(7),
            message: WorkerMessage::Cancel {
                reason: CancelReason::AuthorizationChanged,
            },
        };
        let encoded = serde_json::to_vec(&frame).unwrap();
        let decoded: WorkerFrame = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, frame);

        let unknown = br#"{"protocol_version":2,"sequence":"3","operation_id":"7","message":{"type":"cancel","reason":"owner_changed"}}"#;
        assert!(serde_json::from_slice::<WorkerFrame>(unknown).is_err());
    }

    #[test]
    fn hello_accepts_only_the_opaque_bounded_parent_grant() {
        let valid = br#"{"protocol_version":2,"sequence":"1","operation_id":"0","message":{"type":"hello","challenge":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","parent_grant":[1,2,3]}}"#;
        let frame: WorkerFrame = serde_json::from_slice(valid).unwrap();
        assert!(frame.validate().is_ok());

        let caller_claim = br#"{"protocol_version":2,"sequence":"1","operation_id":"0","message":{"type":"hello","challenge":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","parent_grant":[1],"parent_pid":42}}"#;
        assert!(serde_json::from_slice::<WorkerFrame>(caller_claim).is_err());

        let oversize = WorkerMessage::Hello {
            challenge: "a".repeat(64),
            parent_grant: vec![0; MAX_PARENT_GRANT_BYTES + 1],
        };
        assert!(oversize.validate().is_err());
    }

    #[test]
    fn completion_metadata_is_bounded_and_never_repeats_the_answer() {
        let valid = br#"{"protocol_version":2,"sequence":"3","operation_id":"2","message":{"type":"llm_complete","output_bytes":4,"output_tokens":1,"input_tokens":8,"verified_template_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","answer_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}"#;
        let frame: WorkerFrame = serde_json::from_slice(valid).unwrap();
        assert!(frame.validate().is_ok());

        let duplicated_answer = br#"{"protocol_version":2,"sequence":"3","operation_id":"2","message":{"type":"llm_complete","text":"answer","output_bytes":4,"output_tokens":1,"input_tokens":8,"verified_template_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","answer_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}"#;
        assert!(serde_json::from_slice::<WorkerFrame>(duplicated_answer).is_err());

        let invalid_digest = br#"{"protocol_version":2,"sequence":"3","operation_id":"2","message":{"type":"llm_complete","output_bytes":4,"output_tokens":1,"input_tokens":8,"verified_template_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","answer_sha256":"not-a-sha256"}}"#;
        assert!(serde_json::from_slice::<WorkerFrame>(invalid_digest)
            .unwrap()
            .validate()
            .is_err());
    }
}
