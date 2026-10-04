use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{self, Read, Write};
use thiserror::Error;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 65_536;
pub const MAX_CHUNK_BYTES: usize = 4_096;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("I/O failure: {0}")]
    Io(#[from] io::Error),
    #[error("invalid frame length: {0}")]
    InvalidLength(usize),
    #[error("truncated frame")]
    Truncated,
    #[error("clean end of stream")]
    EndOfStream,
    #[error("frame is not UTF-8")]
    InvalidUtf8,
    #[error("frame is not valid protocol JSON")]
    InvalidJson,
    #[error("invalid protocol message: {0}")]
    InvalidMessage(String),
}

pub trait WireMessage {
    fn validate(&self) -> Result<(), ProtocolError>;
    fn validate_json_shape(value: &serde_json::Value) -> Result<(), ProtocolError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestEnvelope {
    pub protocol_version: u16,
    pub request_id: u64,
    pub request: Request,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Hello {
        manifest_sha256: String,
        run_id: String,
        generation_id: String,
    },
    Probe {
        probe: ProbeKind,
    },
    Cancel {
        target_request_id: u64,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKind {
    Deterministic,
    Health,
    Cancellation,
    NetworkAttempt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseEnvelope {
    pub protocol_version: u16,
    pub request_id: u64,
    pub response: Response,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Ready {
        helper_build_id: String,
        run_id: String,
        generation_id: String,
    },
    Started,
    Chunk {
        sequence: u32,
        text: String,
    },
    Cancelled,
    Complete {
        output_bytes: u64,
        output_sha256: String,
    },
    Error {
        code: HelperErrorCode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperErrorCode {
    InvalidRequest,
    Cancelled,
    Internal,
}

impl WireMessage for RequestEnvelope {
    fn validate(&self) -> Result<(), ProtocolError> {
        validate_header(self.protocol_version, self.request_id)?;
        match &self.request {
            Request::Hello {
                manifest_sha256,
                run_id,
                generation_id,
            } => {
                validate_sha256(manifest_sha256)?;
                validate_nonce(run_id)?;
                validate_nonce(generation_id)
            }
            Request::Probe { .. } | Request::Shutdown => Ok(()),
            Request::Cancel { target_request_id } if *target_request_id == 0 => {
                Err(invalid("cancel target request ID must be non-zero"))
            }
            Request::Cancel { .. } => Ok(()),
        }
    }

    fn validate_json_shape(value: &serde_json::Value) -> Result<(), ProtocolError> {
        require_exact_keys(value, &["protocol_version", "request_id", "request"])?;
        let request = value
            .get("request")
            .ok_or_else(|| invalid("request object missing"))?;
        match request.get("type").and_then(serde_json::Value::as_str) {
            Some("hello") => require_exact_keys(
                request,
                &["type", "manifest_sha256", "run_id", "generation_id"],
            ),
            Some("probe") => require_exact_keys(request, &["type", "probe"]),
            Some("cancel") => require_exact_keys(request, &["type", "target_request_id"]),
            Some("shutdown") => require_exact_keys(request, &["type"]),
            _ => Err(invalid("unknown request type")),
        }
    }
}

impl WireMessage for ResponseEnvelope {
    fn validate(&self) -> Result<(), ProtocolError> {
        validate_header(self.protocol_version, self.request_id)?;
        match &self.response {
            Response::Ready {
                helper_build_id,
                run_id,
                generation_id,
            } => {
                validate_identifier(helper_build_id)?;
                validate_nonce(run_id)?;
                validate_nonce(generation_id)
            }
            Response::Chunk { text, .. } if text.len() > MAX_CHUNK_BYTES => {
                Err(invalid("chunk exceeds byte limit"))
            }
            Response::Complete { output_sha256, .. } => validate_sha256(output_sha256),
            _ => Ok(()),
        }
    }

    fn validate_json_shape(value: &serde_json::Value) -> Result<(), ProtocolError> {
        require_exact_keys(value, &["protocol_version", "request_id", "response"])?;
        let response = value
            .get("response")
            .ok_or_else(|| invalid("response object missing"))?;
        match response.get("type").and_then(serde_json::Value::as_str) {
            Some("ready") => require_exact_keys(
                response,
                &["type", "helper_build_id", "run_id", "generation_id"],
            ),
            Some("started") | Some("cancelled") => require_exact_keys(response, &["type"]),
            Some("chunk") => require_exact_keys(response, &["type", "sequence", "text"]),
            Some("complete") => {
                require_exact_keys(response, &["type", "output_bytes", "output_sha256"])
            }
            Some("error") => require_exact_keys(response, &["type", "code"]),
            _ => Err(invalid("unknown response type")),
        }
    }
}

fn require_exact_keys(value: &serde_json::Value, expected: &[&str]) -> Result<(), ProtocolError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("protocol value must be an object"))?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(invalid("protocol object contains unexpected fields"));
    }
    Ok(())
}

fn validate_header(version: u16, request_id: u64) -> Result<(), ProtocolError> {
    if version != PROTOCOL_VERSION {
        return Err(invalid("unsupported protocol version"));
    }
    if request_id == 0 {
        return Err(invalid("request ID must be non-zero"));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), ProtocolError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(invalid(
            "SHA-256 must be 64 lowercase hexadecimal characters",
        ))
    }
}

fn validate_identifier(value: &str) -> Result<(), ProtocolError> {
    if !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(invalid("helper build ID is invalid"))
    }
}

fn validate_nonce(value: &str) -> Result<(), ProtocolError> {
    if value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(invalid("run or generation ID is invalid"))
    }
}

fn invalid(message: &str) -> ProtocolError {
    ProtocolError::InvalidMessage(message.to_owned())
}

pub fn read_frame<R, T>(reader: &mut R) -> Result<T, ProtocolError>
where
    R: Read,
    T: DeserializeOwned + WireMessage,
{
    let mut length_bytes = [0_u8; 4];
    loop {
        match reader.read(&mut length_bytes[..1]) {
            Ok(0) => return Err(ProtocolError::EndOfStream),
            Ok(1) => break,
            Ok(_) => unreachable!("single-byte read returned more than one byte"),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(ProtocolError::Io(error)),
        }
    }
    read_exact_or_truncated(reader, &mut length_bytes[1..])?;
    let length = u32::from_be_bytes(length_bytes) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(ProtocolError::InvalidLength(length));
    }

    let mut body = vec![0_u8; length];
    read_exact_or_truncated(reader, &mut body)?;
    let text = std::str::from_utf8(&body).map_err(|_| ProtocolError::InvalidUtf8)?;
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| ProtocolError::InvalidJson)?;
    T::validate_json_shape(&value)?;
    let message: T = serde_json::from_value(value).map_err(|_| ProtocolError::InvalidJson)?;
    message.validate()?;
    Ok(message)
}

pub fn write_frame<W, T>(writer: &mut W, message: &T) -> Result<(), ProtocolError>
where
    W: Write,
    T: Serialize + WireMessage,
{
    message.validate()?;
    let value = serde_json::to_value(message).map_err(|_| ProtocolError::InvalidJson)?;
    T::validate_json_shape(&value)?;
    let body = serde_json::to_vec(&value).map_err(|_| ProtocolError::InvalidJson)?;
    if body.is_empty() || body.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::InvalidLength(body.len()));
    }
    writer.write_all(&(body.len() as u32).to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

fn read_exact_or_truncated<R: Read>(
    reader: &mut R,
    buffer: &mut [u8],
) -> Result<(), ProtocolError> {
    match reader.read_exact(buffer) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Err(ProtocolError::Truncated),
        Err(error) => Err(ProtocolError::Io(error)),
    }
}

#[derive(Debug, Default)]
pub struct ChunkSequence {
    next: u32,
}

impl ChunkSequence {
    pub fn observe(&mut self, sequence: u32) -> Result<(), ProtocolError> {
        if sequence != self.next {
            return Err(invalid("chunk sequence is not monotonic"));
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| invalid("chunk sequence overflow"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn framed(body: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(4 + body.len());
        bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn round_trips_hello_request() {
        let request = RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: 1,
            request: Request::Hello {
                manifest_sha256: "a".repeat(64),
                run_id: "b".repeat(32),
                generation_id: "c".repeat(32),
            },
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &request).unwrap();
        let decoded: RequestEnvelope = read_frame(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn rejects_zero_length_frame() {
        let error =
            read_frame::<_, RequestEnvelope>(&mut Cursor::new(0_u32.to_be_bytes())).unwrap_err();
        assert!(matches!(error, ProtocolError::InvalidLength(0)));
    }

    #[test]
    fn rejects_oversized_frame_before_allocation() {
        let length = (MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
        let error = read_frame::<_, RequestEnvelope>(&mut Cursor::new(length)).unwrap_err();
        assert!(matches!(error, ProtocolError::InvalidLength(_)));
    }

    #[test]
    fn rejects_truncated_body() {
        let error =
            read_frame::<_, RequestEnvelope>(&mut Cursor::new(Vec::<u8>::new())).unwrap_err();
        assert!(matches!(error, ProtocolError::EndOfStream));

        let error = read_frame::<_, RequestEnvelope>(&mut Cursor::new(vec![0_u8])).unwrap_err();
        assert!(matches!(error, ProtocolError::Truncated));

        let error = read_frame::<_, RequestEnvelope>(&mut Cursor::new(framed(b"{}"))).unwrap_err();
        assert!(matches!(error, ProtocolError::InvalidMessage(_)));

        let mut truncated = 10_u32.to_be_bytes().to_vec();
        truncated.extend_from_slice(b"{}");
        let error = read_frame::<_, RequestEnvelope>(&mut Cursor::new(truncated)).unwrap_err();
        assert!(matches!(error, ProtocolError::Truncated));
    }

    #[test]
    fn rejects_invalid_utf8_and_json() {
        let error =
            read_frame::<_, RequestEnvelope>(&mut Cursor::new(framed(&[0xff]))).unwrap_err();
        assert!(matches!(error, ProtocolError::InvalidUtf8));

        let error = read_frame::<_, RequestEnvelope>(&mut Cursor::new(framed(b"{"))).unwrap_err();
        assert!(matches!(error, ProtocolError::InvalidJson));
    }

    #[test]
    fn rejects_unknown_fields_and_protocol_version() {
        let unknown =
            br#"{"protocol_version":1,"request_id":1,"request":{"type":"shutdown"},"extra":true}"#;
        assert!(matches!(
            read_frame::<_, RequestEnvelope>(&mut Cursor::new(framed(unknown))).unwrap_err(),
            ProtocolError::InvalidMessage(_)
        ));

        let nested_request =
            br#"{"protocol_version":1,"request_id":1,"request":{"type":"shutdown","extra":true}}"#;
        assert!(matches!(
            read_frame::<_, RequestEnvelope>(&mut Cursor::new(framed(nested_request))).unwrap_err(),
            ProtocolError::InvalidMessage(_)
        ));

        let nested_response =
            br#"{"protocol_version":1,"request_id":1,"response":{"type":"started","extra":true}}"#;
        assert!(matches!(
            read_frame::<_, ResponseEnvelope>(&mut Cursor::new(framed(nested_response)))
                .unwrap_err(),
            ProtocolError::InvalidMessage(_)
        ));

        let wrong_version = RequestEnvelope {
            protocol_version: PROTOCOL_VERSION + 1,
            request_id: 1,
            request: Request::Shutdown,
        };
        let mut bytes = Vec::new();
        let error = write_frame(&mut bytes, &wrong_version).unwrap_err();
        assert!(matches!(error, ProtocolError::InvalidMessage(_)));
    }

    #[test]
    fn rejects_zero_request_id() {
        let request = RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: 0,
            request: Request::Shutdown,
        };
        let error = write_frame(&mut Vec::new(), &request).unwrap_err();
        assert!(matches!(error, ProtocolError::InvalidMessage(_)));
    }

    #[test]
    fn rejects_non_monotonic_chunks() {
        let mut sequence = ChunkSequence::default();
        sequence.observe(0).unwrap();
        assert!(matches!(
            sequence.observe(2).unwrap_err(),
            ProtocolError::InvalidMessage(_)
        ));
    }
}
