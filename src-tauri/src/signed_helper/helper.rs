use crate::helper::protocol::{
    read_frame, write_frame, HelperErrorCode, ProbeKind, ProtocolError, Request, RequestEnvelope,
    Response, ResponseEnvelope, PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use thiserror::Error;

pub trait ParentVerifier {
    fn parent_is_authorized(&self, parent_pid: i32) -> bool;
}

pub struct TeamParentVerifier;

impl ParentVerifier for TeamParentVerifier {
    fn parent_is_authorized(&self, parent_pid: i32) -> bool {
        matches!(
            crate::platform::code_identity::process_matches_current_team(parent_pid),
            Ok(true)
        )
    }
}

pub const HELPER_BUILD_ID: &str = match option_env!("JARVIS_HELPER_BUILD_ID") {
    Some(value) => value,
    None => "unsigned-development",
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HelperCoreError {
    #[error("parent identity is unauthorized")]
    UnauthorizedParent,
    #[error("helper protocol failed")]
    Protocol,
    #[error("request identifiers are not strictly increasing")]
    RequestOrder,
    #[error("handshake is invalid")]
    Handshake,
}

pub fn run_helper(
    reader: &mut impl Read,
    writer: &mut impl Write,
    parent_pid: i32,
    verifier: &impl ParentVerifier,
    build_id: &str,
) -> Result<(), HelperCoreError> {
    if !verifier.parent_is_authorized(parent_pid) {
        return Err(HelperCoreError::UnauthorizedParent);
    }

    let hello: RequestEnvelope = read_frame(reader).map_err(|_| HelperCoreError::Handshake)?;
    let (run_id, generation_id) = match hello.request {
        Request::Hello {
            manifest_sha256: _,
            run_id,
            generation_id,
        } if hello.request_id == 1 => (run_id, generation_id),
        _ => return Err(HelperCoreError::Handshake),
    };
    respond(
        writer,
        hello.request_id,
        Response::Ready {
            helper_build_id: build_id.to_owned(),
            run_id,
            generation_id,
        },
    )?;

    let mut last_request_id = hello.request_id;
    let mut active_cancellation_probe = None;
    loop {
        let envelope: RequestEnvelope = match read_frame(reader) {
            Ok(envelope) => envelope,
            Err(ProtocolError::EndOfStream) => return Ok(()),
            Err(_) => return Err(HelperCoreError::Protocol),
        };
        if envelope.request_id <= last_request_id {
            return Err(HelperCoreError::RequestOrder);
        }
        last_request_id = envelope.request_id;

        if active_cancellation_probe.is_some() && matches!(&envelope.request, Request::Probe { .. })
        {
            respond_error(writer, envelope.request_id)?;
            continue;
        }

        match envelope.request {
            Request::Hello { .. } => return Err(HelperCoreError::Handshake),
            Request::Probe {
                probe: ProbeKind::Health,
            } => {
                respond(writer, envelope.request_id, Response::Started)?;
                let output = b"healthy";
                respond(
                    writer,
                    envelope.request_id,
                    Response::Chunk {
                        sequence: 0,
                        text: String::from_utf8_lossy(output).into_owned(),
                    },
                )?;
                respond(
                    writer,
                    envelope.request_id,
                    Response::Complete {
                        output_bytes: output.len() as u64,
                        output_sha256: format!("{:x}", Sha256::digest(output)),
                    },
                )?;
            }
            Request::Probe {
                probe: ProbeKind::Cancellation,
            } => {
                if active_cancellation_probe.is_some() {
                    respond_error(writer, envelope.request_id)?;
                } else {
                    active_cancellation_probe = Some(envelope.request_id);
                    respond(writer, envelope.request_id, Response::Started)?;
                }
            }
            Request::Probe {
                probe: ProbeKind::Deterministic | ProbeKind::NetworkAttempt,
            } => respond_error(writer, envelope.request_id)?,
            Request::Cancel { target_request_id } => {
                if active_cancellation_probe == Some(target_request_id) {
                    active_cancellation_probe = None;
                    respond(writer, target_request_id, Response::Cancelled)?;
                } else {
                    respond_error(writer, envelope.request_id)?;
                }
            }
            Request::Shutdown => return Ok(()),
        }
    }
}

fn respond(
    writer: &mut impl Write,
    request_id: u64,
    response: Response,
) -> Result<(), HelperCoreError> {
    write_frame(
        writer,
        &ResponseEnvelope {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            response,
        },
    )
    .map_err(|_| HelperCoreError::Protocol)
}

fn respond_error(writer: &mut impl Write, request_id: u64) -> Result<(), HelperCoreError> {
    respond(
        writer,
        request_id,
        Response::Error {
            code: HelperErrorCode::InvalidRequest,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helper::protocol::{
        read_frame, write_frame, ProbeKind, Request, RequestEnvelope, Response, ResponseEnvelope,
        PROTOCOL_VERSION,
    };
    use std::io::Cursor;

    struct Peer(bool);

    impl ParentVerifier for Peer {
        fn parent_is_authorized(&self, _parent_pid: i32) -> bool {
            self.0
        }
    }

    fn request(id: u64, request: Request) -> RequestEnvelope {
        RequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: id,
            request,
        }
    }

    fn input(requests: Vec<RequestEnvelope>) -> Vec<u8> {
        let mut bytes = Vec::new();
        for request in requests {
            write_frame(&mut bytes, &request).unwrap();
        }
        bytes
    }

    fn hello() -> RequestEnvelope {
        request(
            1,
            Request::Hello {
                manifest_sha256: "a".repeat(64),
                run_id: "b".repeat(32),
                generation_id: "c".repeat(32),
            },
        )
    }

    fn responses(bytes: Vec<u8>) -> Vec<ResponseEnvelope> {
        let mut cursor = Cursor::new(bytes);
        let mut output = Vec::new();
        while (cursor.position() as usize) < cursor.get_ref().len() {
            output.push(read_frame(&mut cursor).unwrap());
        }
        output
    }

    #[test]
    fn rejects_parent_before_reading_or_writing_protocol_data() {
        let mut output = Vec::new();
        let error = run_helper(
            &mut Cursor::new(input(vec![hello()])),
            &mut output,
            10,
            &Peer(false),
            "signed-helper-test",
        )
        .unwrap_err();
        assert_eq!(error, HelperCoreError::UnauthorizedParent);
        assert!(output.is_empty());
    }

    #[test]
    fn eof_and_malformed_frames_fail_closed_at_the_correct_state() {
        let mut output = Vec::new();
        assert_eq!(
            run_helper(
                &mut Cursor::new(Vec::<u8>::new()),
                &mut output,
                10,
                &Peer(true),
                "signed-helper-test",
            )
            .unwrap_err(),
            HelperCoreError::Handshake
        );

        let mut output = Vec::new();
        run_helper(
            &mut Cursor::new(input(vec![hello()])),
            &mut output,
            10,
            &Peer(true),
            "signed-helper-test",
        )
        .unwrap();
        assert!(matches!(
            responses(output)[0].response,
            Response::Ready { .. }
        ));

        let mut malformed = input(vec![hello()]);
        malformed.extend_from_slice(&1_u32.to_be_bytes());
        malformed.push(b'{');
        let mut output = Vec::new();
        assert_eq!(
            run_helper(
                &mut Cursor::new(malformed),
                &mut output,
                10,
                &Peer(true),
                "signed-helper-test",
            )
            .unwrap_err(),
            HelperCoreError::Protocol
        );
    }

    #[test]
    fn completes_strict_health_and_shutdown_lifecycle() {
        let requests = input(vec![
            hello(),
            request(
                2,
                Request::Probe {
                    probe: ProbeKind::Health,
                },
            ),
            request(3, Request::Shutdown),
        ]);
        let mut output = Vec::new();
        run_helper(
            &mut Cursor::new(requests),
            &mut output,
            10,
            &Peer(true),
            "signed-helper-test",
        )
        .unwrap();
        let output = responses(output);
        assert!(matches!(output[0].response, Response::Ready { .. }));
        assert!(matches!(output[1].response, Response::Started));
        assert!(matches!(output[2].response, Response::Chunk { .. }));
        assert!(matches!(output[3].response, Response::Complete { .. }));
        assert_eq!(output.len(), 4);
    }

    #[test]
    fn acknowledges_only_the_active_cancellation_probe() {
        let requests = input(vec![
            hello(),
            request(
                2,
                Request::Probe {
                    probe: ProbeKind::Cancellation,
                },
            ),
            request(
                3,
                Request::Probe {
                    probe: ProbeKind::Health,
                },
            ),
            request(
                4,
                Request::Cancel {
                    target_request_id: 2,
                },
            ),
            request(5, Request::Shutdown),
        ]);
        let mut output = Vec::new();
        run_helper(
            &mut Cursor::new(requests),
            &mut output,
            10,
            &Peer(true),
            "signed-helper-test",
        )
        .unwrap();
        let output = responses(output);
        assert!(matches!(output[1].response, Response::Started));
        assert_eq!(output[2].request_id, 3);
        assert!(matches!(output[2].response, Response::Error { .. }));
        assert_eq!(output[3].request_id, 2);
        assert!(matches!(output[3].response, Response::Cancelled));
    }

    #[test]
    fn rejects_network_probe_and_duplicate_request_ids() {
        let requests = input(vec![
            hello(),
            request(
                2,
                Request::Probe {
                    probe: ProbeKind::NetworkAttempt,
                },
            ),
            request(2, Request::Shutdown),
        ]);
        let mut output = Vec::new();
        let error = run_helper(
            &mut Cursor::new(requests),
            &mut output,
            10,
            &Peer(true),
            "signed-helper-test",
        )
        .unwrap_err();
        assert_eq!(error, HelperCoreError::RequestOrder);
        assert!(matches!(
            responses(output)[1].response,
            Response::Error { .. }
        ));
    }
}
