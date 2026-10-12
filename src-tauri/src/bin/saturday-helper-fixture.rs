use saturday::helper::manifest::{
    MAX_CHUNKS, MAX_PEAK_RSS_BYTES, MAX_PROCESS_COUNT, MAX_STDERR_BYTES,
};
use saturday::helper::protocol::{
    read_frame, write_frame, HelperErrorCode, ProtocolError, Request, RequestEnvelope, Response,
    ResponseEnvelope, MAX_CHUNK_BYTES, PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};
use std::io::{self, Write};
use std::net::TcpStream;
use std::process::{Command, ExitCode};
use std::thread;
use std::time::Duration;

const BUILD_ID: &str = "fixture-1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    CancelDelay,
    CancelLate,
    CancelIgnore,
    Malformed,
    Crash,
    StderrFlood,
    StderrOverflow,
    StderrContinuous,
    OutputFlood,
    ValidBurst,
    ResponseFlood,
    RssSpike,
    ProcessFlood,
    ProcessPulse,
    ForkAttempt,
    NetworkAttempt(u16),
    Descendant,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(70),
    }
}

fn run() -> Result<(), ProtocolError> {
    let mode = parse_mode().ok_or_else(|| invalid("invalid fixture mode"))?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    let hello: RequestEnvelope = read_frame(&mut reader)?;
    let (run_id, generation_id) = match hello.request {
        Request::Hello {
            run_id,
            generation_id,
            ..
        } => (run_id, generation_id),
        _ => return Err(invalid("hello required")),
    };
    respond(
        &mut writer,
        hello.request_id,
        Response::Ready {
            helper_build_id: BUILD_ID.to_owned(),
            run_id,
            generation_id,
        },
    )?;

    loop {
        let envelope: RequestEnvelope = read_frame(&mut reader)?;
        match envelope.request {
            Request::Probe { .. } => {
                respond(&mut writer, envelope.request_id, Response::Started)?;
                match mode {
                    Mode::CancelDelay | Mode::CancelLate | Mode::CancelIgnore => {
                        let cancel: RequestEnvelope = read_frame(&mut reader)?;
                        let Request::Cancel { target_request_id } = cancel.request else {
                            return Err(invalid("cancel required"));
                        };
                        if target_request_id != envelope.request_id {
                            return Err(invalid("cancel target mismatch"));
                        }
                        if mode == Mode::CancelIgnore {
                            thread::sleep(Duration::from_secs(60));
                        } else {
                            thread::sleep(Duration::from_millis(25));
                            respond(&mut writer, target_request_id, Response::Cancelled)?;
                            if mode == Mode::CancelLate {
                                respond(
                                    &mut writer,
                                    target_request_id,
                                    Response::Chunk {
                                        sequence: 0,
                                        text: "late".to_owned(),
                                    },
                                )?;
                            }
                        }
                    }
                    Mode::Malformed => {
                        writer.write_all(&1_u32.to_be_bytes())?;
                        writer.write_all(&[0xff])?;
                        writer.flush()?;
                    }
                    Mode::Crash => return Err(invalid("intentional fixture crash")),
                    Mode::StderrFlood => {
                        let mut stderr = io::stderr().lock();
                        let block = [b'x'; 8 * 1024];
                        for _ in 0..128 {
                            stderr.write_all(&block)?;
                        }
                        stderr.flush()?;
                        complete(&mut writer, envelope.request_id, "stderr-drained")?;
                    }
                    Mode::StderrOverflow => {
                        let mut stderr = io::stderr().lock();
                        let block = [b'x'; 8 * 1024];
                        for _ in 0..(MAX_STDERR_BYTES / block.len() as u64) {
                            stderr.write_all(&block)?;
                        }
                        stderr.write_all(b"x")?;
                        stderr.flush()?;
                        complete(&mut writer, envelope.request_id, "stderr-overflow")?;
                    }
                    Mode::StderrContinuous => {
                        let mut stderr = io::stderr().lock();
                        let block = [b'x'; 8 * 1024];
                        while stderr.write_all(&block).is_ok() {}
                        thread::sleep(Duration::from_secs(60));
                    }
                    Mode::OutputFlood => {
                        let text = "x".repeat(MAX_CHUNK_BYTES);
                        for sequence in 0..=MAX_CHUNKS {
                            respond(
                                &mut writer,
                                envelope.request_id,
                                Response::Chunk {
                                    sequence,
                                    text: text.clone(),
                                },
                            )?;
                        }
                    }
                    Mode::ValidBurst => {
                        for sequence in 0..MAX_CHUNKS {
                            respond(
                                &mut writer,
                                envelope.request_id,
                                Response::Chunk {
                                    sequence,
                                    text: "x".to_owned(),
                                },
                            )?;
                        }
                        respond(
                            &mut writer,
                            envelope.request_id,
                            Response::Complete {
                                output_bytes: MAX_CHUNKS as u64,
                                output_sha256: hex_digest(Sha256::digest(vec![
                                    b'x';
                                    MAX_CHUNKS as usize
                                ])),
                            },
                        )?;
                    }
                    Mode::ResponseFlood => {
                        for sequence in 0..(MAX_CHUNKS + 100) {
                            respond(
                                &mut writer,
                                envelope.request_id,
                                Response::Chunk {
                                    sequence,
                                    text: "flood".to_owned(),
                                },
                            )?;
                        }
                        thread::sleep(Duration::from_secs(60));
                    }
                    Mode::RssSpike => {
                        let mut allocation =
                            vec![0_u8; (MAX_PEAK_RSS_BYTES + 32 * 1024 * 1024) as usize];
                        for offset in (0..allocation.len()).step_by(4_096) {
                            allocation[offset] = 1;
                        }
                        std::hint::black_box(&allocation);
                        thread::sleep(Duration::from_millis(500));
                        complete(&mut writer, envelope.request_id, "rss-spike")?;
                    }
                    Mode::ProcessFlood => {
                        for _ in 0..=MAX_PROCESS_COUNT {
                            Command::new("/bin/sleep")
                                .arg("60")
                                .spawn()
                                .map_err(ProtocolError::Io)?;
                        }
                        complete(&mut writer, envelope.request_id, "process-flood")?;
                    }
                    Mode::ProcessPulse => {
                        let mut children = Vec::new();
                        for _ in 0..=MAX_PROCESS_COUNT {
                            children.push(
                                Command::new("/bin/sleep")
                                    .arg("0.2")
                                    .spawn()
                                    .map_err(ProtocolError::Io)?,
                            );
                        }
                        for mut child in children {
                            let _ = child.wait();
                        }
                        complete(&mut writer, envelope.request_id, "process-pulse")?;
                    }
                    Mode::ForkAttempt => {
                        let output = match Command::new("/bin/sleep").arg("60").spawn() {
                            Ok(mut child) => {
                                let _ = child.kill();
                                let _ = child.wait();
                                "fork-allowed"
                            }
                            Err(error)
                                if error.raw_os_error().is_some_and(|code| {
                                    code == libc::EPERM
                                        || code == libc::EACCES
                                        || code == libc::EAGAIN
                                }) =>
                            {
                                "fork-denied-policy"
                            }
                            Err(_) => "fork-error-other",
                        };
                        complete(&mut writer, envelope.request_id, output)?;
                    }
                    Mode::NetworkAttempt(port) => {
                        let output = match TcpStream::connect(("127.0.0.1", port)) {
                            Ok(stream) => {
                                drop(stream);
                                "network-allowed"
                            }
                            Err(error)
                                if error.raw_os_error().is_some_and(|code| {
                                    code == libc::EPERM || code == libc::EACCES
                                }) =>
                            {
                                "network-denied-policy"
                            }
                            Err(_) => "network-error-other",
                        };
                        complete(&mut writer, envelope.request_id, output)?;
                    }
                    Mode::Descendant => {
                        Command::new("/bin/sleep")
                            .arg("60")
                            .spawn()
                            .map_err(ProtocolError::Io)?;
                        complete(&mut writer, envelope.request_id, "descendant-started")?;
                    }
                    Mode::Normal => complete(&mut writer, envelope.request_id, "fixture-output")?,
                }
            }
            Request::Cancel { target_request_id } => {
                respond(&mut writer, target_request_id, Response::Cancelled)?;
            }
            Request::Shutdown => break,
            Request::Hello { .. } => {
                respond(
                    &mut writer,
                    envelope.request_id,
                    Response::Error {
                        code: HelperErrorCode::InvalidRequest,
                    },
                )?;
            }
        }
    }
    Ok(())
}

fn complete(writer: &mut impl Write, request_id: u64, output: &str) -> Result<(), ProtocolError> {
    respond(
        writer,
        request_id,
        Response::Chunk {
            sequence: 0,
            text: output.to_owned(),
        },
    )?;
    respond(
        writer,
        request_id,
        Response::Complete {
            output_bytes: output.len() as u64,
            output_sha256: hex_digest(Sha256::digest(output.as_bytes())),
        },
    )
}

fn respond(
    writer: &mut impl Write,
    request_id: u64,
    response: Response,
) -> Result<(), ProtocolError> {
    write_frame(
        writer,
        &ResponseEnvelope {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            response,
        },
    )
}

fn parse_mode() -> Option<Mode> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() < 2 || arguments[0] != "--mode" {
        return None;
    }
    if arguments[1] == "network-attempt" {
        if arguments.len() != 4 || arguments[2] != "--port" {
            return None;
        }
        return arguments[3].parse::<u16>().ok().map(Mode::NetworkAttempt);
    }
    if arguments.len() != 2 {
        return None;
    }
    match arguments[1].as_str() {
        "normal" => Some(Mode::Normal),
        "cancel-delay" => Some(Mode::CancelDelay),
        "cancel-late" => Some(Mode::CancelLate),
        "cancel-ignore" => Some(Mode::CancelIgnore),
        "malformed" => Some(Mode::Malformed),
        "crash" => Some(Mode::Crash),
        "stderr-flood" => Some(Mode::StderrFlood),
        "stderr-overflow" => Some(Mode::StderrOverflow),
        "stderr-continuous" => Some(Mode::StderrContinuous),
        "output-flood" => Some(Mode::OutputFlood),
        "valid-burst" => Some(Mode::ValidBurst),
        "response-flood" => Some(Mode::ResponseFlood),
        "rss-spike" => Some(Mode::RssSpike),
        "process-flood" => Some(Mode::ProcessFlood),
        "process-pulse" => Some(Mode::ProcessPulse),
        "fork-attempt" => Some(Mode::ForkAttempt),

        "descendant" => Some(Mode::Descendant),
        _ => None,
    }
}

fn invalid(message: &str) -> ProtocolError {
    ProtocolError::InvalidMessage(message.to_owned())
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
