use crate::platform::code_identity::process_matches_identity;
use crate::voice::contract::{
    AsrError, AsrQualityFlag, AsrResult, ConversationEvent, ConversationFailure, Language,
    LanguageMode, OperationId, OwnedPcm, TranscriptEligibility, TurnContext,
    MAX_MODEL_OUTPUT_BYTES, MAX_MODEL_OUTPUT_TOKENS, WORKER_REAP_DEADLINE,
};
use crate::voice_authorization::{
    ApprovedModelHandle, ApprovedRuntimeClosure, AuthorizedWorkerSession, WorkerRole,
};
pub use saturday_voice_worker_protocol as private_protocol;
use saturday_voice_worker_protocol::{
    read_frame, write_frame, CancelReason, ProtocolError, WireU64, WorkerFailure, WorkerFrame,
    WorkerMessage, WorkerRole as WireRole, MAX_ASR_MILLIS, MAX_LLM_MILLIS, MAX_PARENT_GRANT_BYTES,
};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Seek};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;

use std::sync::atomic::{AtomicBool, Ordering};
const CONTROL_FD: RawFd = 3;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
const WORKER_START_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_ACK_TIMEOUT: Duration = Duration::from_millis(250);
const REAPER_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_PARENT_OUTPUT_BYTES: usize = MAX_MODEL_OUTPUT_BYTES;
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

static WORKER_CLEANUP_FAILED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("worker identity or sealed artifact is invalid")]
    Identity,
    #[error("worker startup handshake failed")]
    Handshake,
    #[error("worker operation was cancelled")]
    Cancelled,
    #[error("voice authorization changed while the worker was running")]
    AuthorizationChanged,
    #[error("worker protocol failed")]
    Protocol,
    #[error("worker operation timed out")]
    Timeout,
    #[error("worker output exceeded a fixed bound")]
    ResourceLimit,
    #[error("worker cancellation or reaping could not be verified")]
    Cleanup,
    #[error("worker launch is unavailable on this platform")]
    Unavailable,
    #[error("worker process I/O failed")]
    Io(#[from] io::Error),
}

struct Incoming {
    frame: WorkerFrame,
    rights: Vec<OwnedFd>,
}

/// A fresh fixed-role process for exactly one explicit model operation. No constructor is
/// called during app startup; model and PCM descriptors are the only operation data grants.
pub struct VoiceWorkerClient {
    role: WorkerRole,
    child: Child,
    control: UnixStream,
    responses: Receiver<Result<Incoming, WorkerError>>,
    reader: Option<JoinHandle<()>>,
    owner: crate::voice::contract::VoiceOwner,
    operation_id: OperationId,
    cancellation: crate::voice::contract::CancelToken,
    send_sequence: u64,
    receive_sequence: u64,
    delta_sequence: u16,
    delta_tokens: u16,
    output_bytes: usize,
    output_text: String,
    credit_outstanding: bool,
    credit_bytes: u16,
    terminal: bool,
    started: bool,
    max_rss_bytes: u64,
    task_identity: u64,
    deadline: Instant,
    expected_template_sha256: Option<String>,
}

impl VoiceWorkerClient {
    pub(crate) fn start_llm(
        closure: &ApprovedRuntimeClosure,
        session: &AuthorizedWorkerSession,
        model: ApprovedModelHandle,
        context: &TurnContext,
        prompt: &str,
        user_text: &str,
        response_language: Language,
    ) -> Result<Self, ConversationFailure> {
        if closure.role() != WorkerRole::Llm
            || model.purpose() != crate::voice::contract::AssetPurpose::LlmWeights
            || model.chat_template_sha256().is_none()
            || !valid_prompt_json(prompt)
        {
            return Err(ConversationFailure::AssetUnavailable);
        }
        if !session.is_usable(context.deadline, false, true) {
            return Err(ConversationFailure::AssetUnavailable);
        }
        closure
            .validate_current_authorization_state()
            .map_err(|_| ConversationFailure::AssetUnavailable)?;
        let model_bytes = model.size_bytes();
        let template_sha = model
            .chat_template_sha256()
            .ok_or(ConversationFailure::AssetUnavailable)?
            .to_owned();
        let mut client =
            Self::spawn(closure, session, &model, context).map_err(map_conversation_error)?;
        let frame = client
            .next_frame(WorkerMessage::StartLlm {
                prompt: prompt.to_owned(),
                user_text: user_text.to_owned(),
                response_language,
                chat_template_sha256: template_sha.clone(),
                remaining_millis: remaining_millis(context.deadline, MAX_LLM_MILLIS),
                max_output_tokens: MAX_MODEL_OUTPUT_TOKENS as u16,
                model_bytes: WireU64(model_bytes),
            })
            .map_err(map_conversation_error)?;
        if !session.is_usable(context.deadline, false, true)
            || closure.validate_current_authorization_state().is_err()
        {
            client
                .abort_before_start()
                .map_err(map_conversation_error)?;
            return Err(ConversationFailure::AssetUnavailable);
        }
        client
            .send(frame, &[model.raw_fd()])
            .map_err(map_conversation_error)?;
        client.wait_started().map_err(map_conversation_error)?;
        client.expected_template_sha256 = Some(template_sha.clone());
        Ok(client)
    }

    pub fn poll_llm(
        &mut self,
        max_delta_bytes: usize,
    ) -> Result<Option<ConversationEvent>, ConversationFailure> {
        if self.role != WorkerRole::Llm || self.terminal {
            return Ok(None);
        }
        if let Some(cancellation_reason) = self.cancellation.cancellation_reason() {
            let authorization_changed = cancellation_reason
                == crate::voice::contract::CancellationReason::AuthorizationChanged;
            self.cancel(if authorization_changed {
                CancelReason::AuthorizationChanged
            } else {
                CancelReason::User
            })
            .map_err(map_conversation_error)?;
            return Err(if authorization_changed {
                ConversationFailure::AssetUnavailable
            } else {
                ConversationFailure::Cancelled
            });
        }
        if Instant::now() >= self.deadline() {
            self.cancel(CancelReason::Deadline)
                .map_err(map_conversation_error)?;
            return Err(ConversationFailure::Timeout);
        }
        if max_delta_bytes > 0 {
            if max_delta_bytes > saturday_voice_worker_protocol::MAX_DELTA_BYTES {
                return Err(ConversationFailure::InvalidOutput);
            }
            let requested_credit = max_delta_bytes as u16;
            if !self.credit_outstanding && requested_credit > self.credit_bytes {
                let credit = self
                    .next_frame(WorkerMessage::Credit {
                        max_delta_bytes: requested_credit,
                    })
                    .map_err(map_conversation_error)?;
                self.send(credit, &[]).map_err(map_conversation_error)?;
                self.credit_outstanding = true;
                self.credit_bytes = requested_credit;
            }
        }
        let incoming = match self.responses.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(incoming)) => incoming,
            Ok(Err(error)) => return Err(map_conversation_error(error)),
            Err(RecvTimeoutError::Timeout) => {
                if Instant::now() >= self.deadline() {
                    self.cancel(CancelReason::Deadline)
                        .map_err(map_conversation_error)?;
                    return Err(ConversationFailure::Timeout);
                }
                self.enforce_rss().map_err(map_conversation_error)?;
                return Ok(None);
            }
            Err(RecvTimeoutError::Disconnected) => return Err(ConversationFailure::WorkerProtocol),
        };
        if let Some(cancellation_reason) = self.cancellation.cancellation_reason() {
            let authorization_changed = cancellation_reason
                == crate::voice::contract::CancellationReason::AuthorizationChanged;
            self.cancel(if authorization_changed {
                CancelReason::AuthorizationChanged
            } else {
                CancelReason::User
            })
            .map_err(map_conversation_error)?;
            return Err(if authorization_changed {
                ConversationFailure::AssetUnavailable
            } else {
                ConversationFailure::Cancelled
            });
        }
        if Instant::now() >= self.deadline() {
            self.cancel(CancelReason::Deadline)
                .map_err(map_conversation_error)?;
            return Err(ConversationFailure::Timeout);
        }
        let message = self.accept(incoming).map_err(map_conversation_error)?;
        if matches!(&message, WorkerMessage::TextDelta { .. }) {
            if let Some(reason) = self.cancellation.cancellation_reason() {
                let authorization_changed =
                    reason == crate::voice::contract::CancellationReason::AuthorizationChanged;
                self.cancel(if authorization_changed {
                    CancelReason::AuthorizationChanged
                } else {
                    CancelReason::User
                })
                .map_err(map_conversation_error)?;
                return Err(if authorization_changed {
                    ConversationFailure::AssetUnavailable
                } else {
                    ConversationFailure::Cancelled
                });
            }
        }
        match message {
            WorkerMessage::TextDelta {
                delta_sequence,
                delta_tokens,
                text,
            } => {
                if !self.credit_outstanding
                    || text.len() > usize::from(self.credit_bytes)
                    || self.delta_sequence.checked_add(1) != Some(delta_sequence)
                    || self.delta_tokens.saturating_add(delta_tokens)
                        > MAX_MODEL_OUTPUT_TOKENS as u16
                    || self.output_bytes.saturating_add(text.len()) > MAX_PARENT_OUTPUT_BYTES
                {
                    self.cancel(CancelReason::ProtocolFailure)
                        .map_err(map_conversation_error)?;
                    return Err(ConversationFailure::WorkerProtocol);
                }
                self.delta_sequence = delta_sequence;
                self.delta_tokens += delta_tokens;
                self.output_bytes += text.len();
                self.output_text.push_str(&text);
                self.credit_outstanding = false;
                self.credit_bytes = 0;
                Ok(Some(ConversationEvent::TextDelta {
                    owner: self.owner.clone(),
                    operation_id: self.operation_id,
                    sequence: u64::from(delta_sequence),
                    text,
                }))
            }
            WorkerMessage::LlmComplete {
                output_bytes,
                output_tokens,
                input_tokens: _,
                verified_template_sha256,
                answer_sha256,
            } => {
                let actual_digest = format!("{:x}", Sha256::digest(self.output_text.as_bytes()));
                if output_tokens != self.delta_tokens
                    || usize::from(output_bytes) != self.output_bytes
                    || output_bytes == 0
                    || output_bytes as usize > MAX_PARENT_OUTPUT_BYTES
                    || self.output_bytes > MAX_PARENT_OUTPUT_BYTES
                    || Some(verified_template_sha256) != self.expected_template_sha256
                    || answer_sha256 != actual_digest
                {
                    self.cancel(CancelReason::ProtocolFailure)
                        .map_err(map_conversation_error)?;
                    return Err(ConversationFailure::WorkerProtocol);
                }
                self.credit_outstanding = false;
                self.credit_bytes = 0;
                self.reap_after_terminal().map_err(map_conversation_error)?;
                if let Some(reason) = self.cancellation.cancellation_reason() {
                    return Err(
                        if reason
                            == crate::voice::contract::CancellationReason::AuthorizationChanged
                        {
                            ConversationFailure::AssetUnavailable
                        } else {
                            ConversationFailure::Cancelled
                        },
                    );
                }
                if Instant::now() >= self.deadline() {
                    return Err(ConversationFailure::Timeout);
                }
                Ok(Some(ConversationEvent::Completed {
                    owner: self.owner.clone(),
                    operation_id: self.operation_id,
                    output_tokens,
                }))
            }
            WorkerMessage::Cancelled {} => {
                self.reap_after_terminal().map_err(map_conversation_error)?;
                Err(
                    if self.cancellation.cancellation_reason()
                        == Some(crate::voice::contract::CancellationReason::AuthorizationChanged)
                    {
                        ConversationFailure::AssetUnavailable
                    } else {
                        ConversationFailure::Cancelled
                    },
                )
            }
            WorkerMessage::Failure { code } => {
                self.reap_after_terminal().map_err(map_conversation_error)?;
                Err(map_worker_failure(code))
            }
            _ => {
                self.cancel(CancelReason::ProtocolFailure)
                    .map_err(map_conversation_error)?;
                Err(ConversationFailure::WorkerProtocol)
            }
        }
    }

    pub fn cancel_llm(
        &mut self,
        context: &TurnContext,
    ) -> Result<crate::voice::contract::CleanupReceipt, ConversationFailure> {
        if self.terminal {
            return Ok(crate::voice::contract::CleanupReceipt::verified(
                crate::voice::contract::CleanupCode::WorkerExited,
            ));
        }
        context.cancel.cancel();
        self.cancel(CancelReason::User)
            .map_err(map_conversation_error)?;
        Ok(crate::voice::contract::CleanupReceipt::verified(
            crate::voice::contract::CleanupCode::WorkerExited,
        ))
    }

    pub(crate) fn transcribe_asr(
        closure: &ApprovedRuntimeClosure,
        session: &AuthorizedWorkerSession,
        model: ApprovedModelHandle,
        pcm: OwnedPcm,
        language_mode: LanguageMode,
        context: &TurnContext,
    ) -> Result<AsrResult, AsrError> {
        if closure.role() != WorkerRole::Asr
            || model.purpose() != crate::voice::contract::AssetPurpose::AsrWeights
        {
            return Err(AsrError::AssetUnavailable);
        }
        if !session.is_usable(context.deadline, true, false) {
            return Err(AsrError::AssetUnavailable);
        }
        closure
            .validate_current_authorization_state()
            .map_err(|_| AsrError::AssetUnavailable)?;
        let sample_count = pcm.sample_count();
        let duration_capped = pcm.duration_capped();
        let pcm_file = pcm.into_file();
        validate_model_fd(model.as_file(), model.size_bytes())
            .map_err(|_| AsrError::AssetUnavailable)?;
        validate_pcm_fd(&pcm_file).map_err(|_| AsrError::InvalidAudio)?;
        let mut client = Self::spawn(closure, session, &model, context).map_err(map_asr_error)?;
        let frame = client
            .next_frame(WorkerMessage::StartAsr {
                sample_count,
                duration_capped,
                language_mode,
                remaining_millis: remaining_millis(context.deadline, MAX_ASR_MILLIS),
                model_bytes: WireU64(model.size_bytes()),
            })
            .map_err(map_asr_error)?;
        if !session.is_usable(context.deadline, true, false)
            || closure.validate_current_authorization_state().is_err()
        {
            client.abort_before_start().map_err(map_asr_error)?;
            return Err(AsrError::AssetUnavailable);
        }
        client
            .send(frame, &[model.raw_fd(), pcm_file.as_raw_fd()])
            .map_err(map_asr_error)?;
        drop(pcm_file);
        client.wait_started().map_err(map_asr_error)?;
        let result = loop {
            let cancellation_reason = context.cancel.cancellation_reason();
            if cancellation_reason.is_some() || Instant::now() >= context.deadline {
                let reason = match cancellation_reason {
                    Some(crate::voice::contract::CancellationReason::AuthorizationChanged) => {
                        CancelReason::AuthorizationChanged
                    }
                    Some(crate::voice::contract::CancellationReason::User) => CancelReason::User,
                    None => CancelReason::Deadline,
                };
                client.cancel(reason).map_err(map_asr_error)?;
                return Err(if reason == CancelReason::AuthorizationChanged {
                    AsrError::AssetUnavailable
                } else if reason == CancelReason::User {
                    AsrError::Cancelled
                } else {
                    AsrError::Timeout
                });
            }
            let incoming = match client.responses.recv_timeout(Duration::from_millis(20)) {
                Ok(Ok(incoming)) => incoming,
                Ok(Err(error)) => return Err(map_asr_error(error)),
                Err(RecvTimeoutError::Timeout) => {
                    client.enforce_rss().map_err(map_asr_error)?;
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return Err(AsrError::WorkerFailed),
            };
            let cancellation_reason = context.cancel.cancellation_reason();
            if cancellation_reason.is_some() || Instant::now() >= context.deadline {
                let reason = match cancellation_reason {
                    Some(crate::voice::contract::CancellationReason::AuthorizationChanged) => {
                        CancelReason::AuthorizationChanged
                    }
                    Some(crate::voice::contract::CancellationReason::User) => CancelReason::User,
                    None => CancelReason::Deadline,
                };
                client.cancel(reason).map_err(map_asr_error)?;
                return Err(if reason == CancelReason::AuthorizationChanged {
                    AsrError::AssetUnavailable
                } else if reason == CancelReason::User {
                    AsrError::Cancelled
                } else {
                    AsrError::Timeout
                });
            }
            let message = client.accept(incoming).map_err(map_asr_error)?;
            match message {
                WorkerMessage::AsrComplete { result_json } => {
                    client.reap_after_terminal().map_err(map_asr_error)?;
                    break serde_json::from_str::<AsrResult>(&result_json)
                        .map_err(|_| AsrError::InvalidEvidence)?;
                }
                WorkerMessage::Cancelled {} => {
                    client.reap_after_terminal().map_err(map_asr_error)?;
                    return Err(
                        if context.cancel.cancellation_reason()
                            == Some(
                                crate::voice::contract::CancellationReason::AuthorizationChanged,
                            )
                        {
                            AsrError::AssetUnavailable
                        } else {
                            AsrError::Cancelled
                        },
                    );
                }
                WorkerMessage::Failure { code } => {
                    client.reap_after_terminal().map_err(map_asr_error)?;
                    return Err(map_asr_failure(code));
                }
                _ => {
                    let _ = client.cancel(CancelReason::ProtocolFailure);
                    return Err(AsrError::WorkerFailed);
                }
            }
        };
        if let Some(reason) = context.cancel.cancellation_reason() {
            return Err(
                if reason == crate::voice::contract::CancellationReason::AuthorizationChanged {
                    AsrError::AssetUnavailable
                } else {
                    AsrError::Cancelled
                },
            );
        }
        if Instant::now() >= context.deadline {
            return Err(AsrError::Timeout);
        }
        let mut result = validate_asr_result(result, language_mode)?;
        if duration_capped {
            if !result.quality_flags.contains(&AsrQualityFlag::Truncated) {
                result.quality_flags.push(AsrQualityFlag::Truncated);
            }
            result.eligibility = TranscriptEligibility::NeedsReview;
        }
        Ok(result)
    }

    fn spawn(
        closure: &ApprovedRuntimeClosure,
        session: &AuthorizedWorkerSession,
        model: &ApprovedModelHandle,
        context: &TurnContext,
    ) -> Result<Self, WorkerError> {
        if WORKER_CLEANUP_FAILED.load(Ordering::Acquire) {
            return Err(WorkerError::Cleanup);
        }
        let model_path = model.sandbox_read_path();
        if context.is_cancelled_or_expired() || closure.binding_sha256() != model.binding_sha256() {
            return Err(WorkerError::Identity);
        }
        let role = closure.role();
        let session_matches_role = match role {
            WorkerRole::Asr => session.is_usable(context.deadline, true, false),
            WorkerRole::Llm => session.is_usable(context.deadline, false, true),
        };
        if !session_matches_role {
            return Err(WorkerError::AuthorizationChanged);
        }
        validate_model_fd(model.as_file(), model.size_bytes())?;
        verify_model_path(model)?;
        verify_worker_artifact(closure)?;
        let sandbox = validate_sandbox_exec()?;
        let (parent_executable, parent_resources) = parent_sandbox_paths()?;
        let profile = build_sandbox_profile(
            closure.executable(),
            model_path,
            &parent_executable,
            &parent_resources,
            closure.parent_resource_files(),
        )?;
        let (parent_socket, child_socket) = UnixStream::pair()?;
        set_nosigpipe(&parent_socket)?;
        set_nosigpipe(&child_socket)?;
        let child_fd = child_socket.as_raw_fd();
        let mut command = Command::new(sandbox);
        command
            .arg("-p")
            .arg(profile)
            .arg("--")
            .arg(closure.executable())
            .arg("--control-fd=3")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: this child-only hook duplicates the fixed control socket and applies fixed
        // process limits before sandbox-exec installs the deny-default profile.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(child_fd, CONTROL_FD) < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::fcntl(CONTROL_FD, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                // Close every non-control descriptor while the child is still unsandboxed.
                let mut inherited_nofile: libc::rlimit = std::mem::zeroed();
                if libc::getrlimit(libc::RLIMIT_NOFILE, &mut inherited_nofile) != 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut fd = (CONTROL_FD + 1) as libc::rlim_t;
                while fd < inherited_nofile.rlim_cur {
                    let result = libc::close(fd as RawFd);
                    if result != 0 {
                        let error = io::Error::last_os_error();
                        if error.raw_os_error() != Some(libc::EBADF) {
                            return Err(error);
                        }
                    }
                    fd += 1;
                }
                let nproc = libc::rlimit {
                    rlim_cur: 1,
                    rlim_max: 1,
                };
                let nofile = libc::rlimit {
                    rlim_cur: 64,
                    rlim_max: 64,
                };
                let core = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::setrlimit(libc::RLIMIT_NPROC, &nproc) != 0
                    || libc::setrlimit(libc::RLIMIT_NOFILE, &nofile) != 0
                    || libc::setrlimit(libc::RLIMIT_CORE, &core) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.env_clear();
        let mut child = command.spawn()?;
        drop(child_socket);
        let task_identity = match spawned_task_identity(child.id()) {
            Ok(identity) => identity,
            Err(error) => {
                terminate_child(&mut child, None)?;
                return Err(error);
            }
        };
        let role_wire = wire_role(role);
        let mut control = parent_socket;
        if let Err(error) = control.set_read_timeout(Some(HANDSHAKE_TIMEOUT)) {
            terminate_child(&mut child, Some(task_identity))?;
            return Err(WorkerError::Io(error));
        }
        let (challenge_frame, rights) = match read_frame(&mut control) {
            Ok(frame) => frame,
            Err(_) => {
                terminate_child(&mut child, Some(task_identity))?;
                return Err(WorkerError::Handshake);
            }
        };
        if !rights.is_empty()
            || challenge_frame.sequence.0 != 1
            || challenge_frame.operation_id.0 != 0
        {
            terminate_child(&mut child, Some(task_identity))?;
            return Err(WorkerError::Handshake);
        }
        let challenge_hex = match challenge_frame.message {
            WorkerMessage::Challenge {
                role: challenge_role,
                challenge,
            } if challenge_role == role_wire => challenge,
            _ => {
                terminate_child(&mut child, Some(task_identity))?;
                return Err(WorkerError::Handshake);
            }
        };
        let pid = match i32::try_from(child.id()) {
            Ok(pid) => pid,
            Err(_) => {
                terminate_child(&mut child, Some(task_identity))?;
                return Err(WorkerError::Identity);
            }
        };
        let current_task_identity = match spawned_task_identity(child.id()) {
            Ok(identity) => identity,
            Err(error) => {
                terminate_child(&mut child, Some(task_identity))?;
                return Err(error);
            }
        };
        if current_task_identity != task_identity
            || !process_matches_identity(
                pid,
                closure.worker_bundle_identifier(),
                closure.worker_team_id_sha256(),
                closure.worker_requirement_sha256(),
            )
            .unwrap_or(false)
        {
            terminate_child(&mut child, Some(task_identity))?;
            return Err(WorkerError::Identity);
        }
        let parent_grant = match session.issue_parent_grant(
            closure.parent_grant_template(),
            &challenge_hex,
            child.id(),
            task_identity,
            &context.cancel,
        ) {
            Ok(grant) if !grant.is_empty() && grant.len() <= MAX_PARENT_GRANT_BYTES => grant,
            _ => {
                terminate_child(&mut child, Some(task_identity))?;
                return Err(WorkerError::Identity);
            }
        };
        let hello = WorkerFrame {
            protocol_version: saturday_voice_worker_protocol::PROTOCOL_VERSION,
            sequence: WireU64(1),
            operation_id: WireU64(0),
            message: WorkerMessage::Hello {
                challenge: challenge_hex.clone(),
                parent_grant,
            },
        };
        if write_frame(&mut control, &hello, &[]).is_err() {
            terminate_child(&mut child, Some(task_identity))?;
            return Err(WorkerError::Protocol);
        }
        let (ready, rights) = match read_frame(&mut control) {
            Ok(ready) => ready,
            Err(_) => {
                terminate_child(&mut child, Some(task_identity))?;
                return Err(WorkerError::Handshake);
            }
        };
        if !rights.is_empty()
            || ready.sequence.0 != 2
            || ready.operation_id.0 != 0
            || !matches!(ready.message, WorkerMessage::Ready { ref role, ref challenge, worker_pid, ref source_seal_sha256, ref resource_seal_sha256 }
                if *role == role_wire
                    && challenge == &challenge_hex
                    && worker_pid == child.id()
                    && source_seal_sha256 == closure.source_seal_sha256()
                    && resource_seal_sha256 == closure.resource_seal_sha256())
        {
            terminate_child(&mut child, Some(task_identity))?;
            return Err(WorkerError::Handshake);
        }
        let final_task_identity = match spawned_task_identity(child.id()) {
            Ok(identity) => identity,
            Err(error) => {
                terminate_child(&mut child, Some(task_identity))?;
                return Err(error);
            }
        };
        if final_task_identity != task_identity
            || !process_matches_identity(
                pid,
                closure.worker_bundle_identifier(),
                closure.worker_team_id_sha256(),
                closure.worker_requirement_sha256(),
            )
            .unwrap_or(false)
        {
            terminate_child(&mut child, Some(task_identity))?;
            return Err(WorkerError::Identity);
        }
        if let Err(error) = control.set_read_timeout(None) {
            terminate_child(&mut child, Some(task_identity))?;
            return Err(WorkerError::Io(error));
        }
        let reader_stream = match control.try_clone() {
            Ok(stream) => stream,
            Err(error) => {
                terminate_child(&mut child, Some(task_identity))?;
                return Err(WorkerError::Io(error));
            }
        };
        let (sender, responses) = mpsc::sync_channel(1);
        let reader = match thread::Builder::new()
            .spawn(move || read_worker_responses(reader_stream, sender))
        {
            Ok(reader) => reader,
            Err(error) => {
                terminate_child(&mut child, Some(task_identity))?;
                return Err(WorkerError::Io(error));
            }
        };
        Ok(Self {
            role,
            child,
            control,
            responses,
            reader: Some(reader),
            owner: context.owner.clone(),
            operation_id: context.operation_id,
            cancellation: context.cancel.clone(),
            send_sequence: 1,
            receive_sequence: 2,
            delta_sequence: 0,
            delta_tokens: 0,
            output_bytes: 0,
            output_text: String::new(),
            credit_outstanding: false,
            credit_bytes: 0,
            terminal: false,
            started: false,
            max_rss_bytes: closure.max_rss_bytes(),
            task_identity,
            deadline: context.deadline,
            expected_template_sha256: None,
        })
    }

    fn abort_before_start(&mut self) -> Result<(), WorkerError> {
        if self.started || self.terminal {
            return Err(WorkerError::Protocol);
        }
        terminate_child(&mut self.child, Some(self.task_identity))?;
        self.join_reader()?;
        self.terminal = true;
        Ok(())
    }

    fn next_frame(&mut self, message: WorkerMessage) -> Result<WorkerFrame, WorkerError> {
        let sequence = self
            .send_sequence
            .checked_add(1)
            .ok_or(WorkerError::Protocol)?;
        self.send_sequence = sequence;
        Ok(WorkerFrame {
            protocol_version: saturday_voice_worker_protocol::PROTOCOL_VERSION,
            sequence: WireU64(sequence),
            operation_id: WireU64(self.operation_id.0),
            message,
        })
    }

    fn send(&mut self, frame: WorkerFrame, rights: &[RawFd]) -> Result<(), WorkerError> {
        write_frame(&mut self.control, &frame, rights).map_err(|_| WorkerError::Protocol)
    }

    fn wait_started(&mut self) -> Result<(), WorkerError> {
        let start_deadline = (Instant::now() + WORKER_START_TIMEOUT).min(self.deadline);
        loop {
            if let Some(reason) = self.cancellation.cancellation_reason() {
                let (wire_reason, error) = match reason {
                    crate::voice::contract::CancellationReason::AuthorizationChanged => (
                        CancelReason::AuthorizationChanged,
                        WorkerError::AuthorizationChanged,
                    ),
                    crate::voice::contract::CancellationReason::User => {
                        (CancelReason::User, WorkerError::Cancelled)
                    }
                };
                self.cancel(wire_reason)?;
                return Err(error);
            }
            let now = Instant::now();
            if now >= self.deadline || now >= start_deadline {
                self.cancel(CancelReason::Deadline)?;
                return Err(WorkerError::Timeout);
            }
            match self
                .responses
                .recv_timeout((start_deadline - now).min(Duration::from_millis(20)))
            {
                Ok(Ok(incoming)) => {
                    if !matches!(self.accept(incoming)?, WorkerMessage::Started {}) {
                        return Err(WorkerError::Handshake);
                    }
                    return Ok(());
                }
                Ok(Err(error)) => return Err(error),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(WorkerError::Handshake),
            }
        }
    }

    fn accept(&mut self, incoming: Incoming) -> Result<WorkerMessage, WorkerError> {
        let frame = incoming.frame;
        if !incoming.rights.is_empty()
            || self.receive_sequence.checked_add(1) != Some(frame.sequence.0)
            || frame.operation_id.0 != self.operation_id.0
        {
            return Err(WorkerError::Protocol);
        }
        let kind_allowed = match (&self.role, &frame.message) {
            (_, WorkerMessage::Started {}) => !self.started,
            (WorkerRole::Asr, WorkerMessage::AsrComplete { .. })
            | (WorkerRole::Asr, WorkerMessage::Cancelled {})
            | (WorkerRole::Asr, WorkerMessage::Failure { .. }) => self.started,
            (WorkerRole::Llm, WorkerMessage::TextDelta { .. })
            | (WorkerRole::Llm, WorkerMessage::LlmComplete { .. })
            | (WorkerRole::Llm, WorkerMessage::Cancelled {})
            | (WorkerRole::Llm, WorkerMessage::Failure { .. }) => self.started,
            _ => false,
        };
        if !kind_allowed {
            return Err(WorkerError::Protocol);
        }
        self.receive_sequence = frame.sequence.0;
        self.started |= matches!(&frame.message, WorkerMessage::Started {});
        Ok(frame.message)
    }

    fn cancel(&mut self, reason: CancelReason) -> Result<(), WorkerError> {
        if self.terminal {
            return Ok(());
        }
        match checked_try_wait(&mut self.child) {
            Ok(Some(status)) => {
                self.join_reader()?;
                self.terminal = true;
                return if status.success() {
                    Ok(())
                } else {
                    Err(WorkerError::Cleanup)
                };
            }
            Ok(None) => {}
            Err(_) => {
                let cleanup = terminate_child(&mut self.child, Some(self.task_identity));
                let joined = self.join_reader();
                if cleanup.is_ok() && joined.is_ok() {
                    self.terminal = true;
                }
                return Err(WorkerError::Cleanup);
            }
        }
        self.cancellation.cancel();
        let cancel = match self.next_frame(WorkerMessage::Cancel { reason }) {
            Ok(cancel) => cancel,
            Err(error) => {
                terminate_child(&mut self.child, Some(self.task_identity))?;
                self.join_reader()?;
                self.terminal = true;
                return Err(error);
            }
        };
        let _ = self.send(cancel, &[]);
        let ack_deadline = Instant::now() + CANCEL_ACK_TIMEOUT;
        loop {
            let remaining = ack_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match self.responses.recv_timeout(remaining) {
                Ok(Ok(incoming)) => match self.accept(incoming) {
                    Ok(WorkerMessage::Cancelled {}) => return self.reap_after_terminal(),
                    Ok(WorkerMessage::Started {}) | Ok(WorkerMessage::TextDelta { .. }) => {}
                    _ => break,
                },
                Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => break,
            }
        }
        terminate_child(&mut self.child, Some(self.task_identity))?;
        self.join_reader()?;
        self.terminal = true;
        Ok(())
    }

    fn reap_after_terminal(&mut self) -> Result<(), WorkerError> {
        match checked_try_wait(&mut self.child) {
            Ok(Some(status)) => {
                self.join_reader()?;
                self.terminal = true;
                return if status.success() {
                    Ok(())
                } else {
                    Err(WorkerError::Cleanup)
                };
            }
            Ok(None) => {}
            Err(_) => {
                let cleanup = terminate_child(&mut self.child, Some(self.task_identity));
                let joined = self.join_reader();
                if cleanup.is_ok() && joined.is_ok() {
                    self.terminal = true;
                }
                return Err(WorkerError::Cleanup);
            }
        }
        let deadline = Instant::now() + WORKER_REAP_DEADLINE;
        while Instant::now() < deadline {
            match checked_try_wait(&mut self.child) {
                Ok(Some(status)) => {
                    self.join_reader()?;
                    self.terminal = true;
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(WorkerError::Cleanup)
                    };
                }
                Ok(None) => {}
                Err(_) => {
                    let cleanup = terminate_child(&mut self.child, Some(self.task_identity));
                    let joined = self.join_reader();
                    if cleanup.is_ok() && joined.is_ok() {
                        self.terminal = true;
                    }
                    return Err(WorkerError::Cleanup);
                }
            }
            thread::sleep(REAPER_POLL_INTERVAL);
        }
        terminate_child(&mut self.child, Some(self.task_identity))?;
        self.join_reader()?;
        self.terminal = true;
        Ok(())
    }

    fn join_reader(&mut self) -> Result<(), WorkerError> {
        if let Some(reader) = self.reader.take() {
            if reader.join().is_err() {
                mark_worker_cleanup_failed();
                return Err(WorkerError::Cleanup);
            }
        }
        Ok(())
    }

    fn enforce_rss(&mut self) -> Result<(), WorkerError> {
        let pid = i32::try_from(self.child.id()).map_err(|_| WorkerError::Identity)?;
        let rss = physical_footprint(pid, self.task_identity)?;
        if rss > self.max_rss_bytes {
            terminate_child(&mut self.child, Some(self.task_identity))?;
            self.join_reader()?;
            self.terminal = true;
            return Err(WorkerError::ResourceLimit);
        }
        Ok(())
    }

    fn deadline(&self) -> Instant {
        self.deadline
    }
}

impl Drop for VoiceWorkerClient {
    fn drop(&mut self) {
        if !self.terminal {
            let _ = self.cancel(CancelReason::Shutdown);
        }
    }
}

fn read_worker_responses(
    mut stream: UnixStream,
    sender: SyncSender<Result<Incoming, WorkerError>>,
) {
    loop {
        match read_frame(&mut stream) {
            Ok((frame, rights)) => {
                if sender.send(Ok(Incoming { frame, rights })).is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = sender.send(Err(map_protocol_error(error)));
                break;
            }
        }
    }
}

fn validate_model_fd(file: &File, expected_size: u64) -> Result<(), WorkerError> {
    let metadata = file.metadata()?;
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    let fd_flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    if !metadata.is_file()
        || metadata.len() != expected_size
        || metadata.nlink() != 1
        || flags < 0
        || flags & libc::O_ACCMODE != libc::O_RDONLY
        || fd_flags < 0
        || fd_flags & libc::FD_CLOEXEC == 0
    {
        return Err(WorkerError::Identity);
    }
    Ok(())
}

fn validate_pcm_fd(file: &File) -> Result<(), WorkerError> {
    let metadata = file.metadata()?;
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    let fd_flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    if !metadata.file_type().is_fifo()
        || flags < 0
        || flags & libc::O_ACCMODE != libc::O_RDONLY
        || fd_flags < 0
        || fd_flags & libc::FD_CLOEXEC == 0
    {
        return Err(WorkerError::Identity);
    }
    Ok(())
}

fn verify_worker_artifact(closure: &ApprovedRuntimeClosure) -> Result<(), WorkerError> {
    let file = closure.executable_file();
    let metadata = file.metadata()?;
    let path_metadata = fs::symlink_metadata(closure.executable())?;
    if !metadata.is_file()
        || metadata.len() != closure.worker_bytes()
        || metadata.nlink() != 1
        || metadata.mode() & 0o222 != 0
        || metadata.mode() & 0o111 == 0
        || path_metadata.dev() != metadata.dev()
        || path_metadata.ino() != metadata.ino()
        || path_metadata.file_type().is_symlink()
        || !path_metadata.is_file()
    {
        return Err(WorkerError::Identity);
    }
    let mut hash = Sha256::new();
    let mut reader = file.try_clone()?;
    reader.seek(std::io::SeekFrom::Start(0))?;
    let copied = io::copy(&mut reader, &mut hash)?;
    if copied != closure.worker_bytes()
        || format!("{:x}", hash.finalize()) != closure.worker_sha256()
    {
        return Err(WorkerError::Identity);
    }
    Ok(())
}

fn verify_model_path(model: &ApprovedModelHandle) -> Result<(), WorkerError> {
    let descriptor_metadata = model.as_file().metadata()?;
    let path_metadata = fs::symlink_metadata(model.sandbox_read_path())?;
    if path_metadata.file_type().is_symlink()
        || !path_metadata.is_file()
        || path_metadata.dev() != descriptor_metadata.dev()
        || path_metadata.ino() != descriptor_metadata.ino()
        || descriptor_metadata.nlink() != 1
    {
        return Err(WorkerError::Identity);
    }
    Ok(())
}

fn validate_sandbox_exec() -> Result<&'static Path, WorkerError> {
    let path = Path::new(SANDBOX_EXEC);
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.uid() != 0 {
        return Err(WorkerError::Unavailable);
    }
    Ok(path)
}

fn parent_sandbox_paths() -> Result<(PathBuf, PathBuf), WorkerError> {
    let executable = std::env::current_exe()?;
    let metadata = fs::symlink_metadata(&executable)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(WorkerError::Identity);
    }
    let executable = fs::canonicalize(executable)?;
    let contents = executable
        .parent()
        .and_then(Path::parent)
        .ok_or(WorkerError::Identity)?;
    let resources = contents.join("Resources/saturday-resources");
    let metadata = fs::symlink_metadata(&resources)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(WorkerError::Identity);
    }
    Ok((executable, fs::canonicalize(resources)?))
}

fn build_sandbox_profile(
    worker: &Path,
    model: &Path,
    parent_executable: &Path,
    parent_resources: &Path,
    parent_resource_files: &[std::path::PathBuf],
) -> Result<String, WorkerError> {
    let worker = sbpl_literal(worker)?;
    let model = sbpl_literal(model)?;
    let parent_executable = sbpl_literal(parent_executable)?;
    let mut resource_literals = String::new();
    for path in parent_resource_files {
        if !path.is_absolute() || !path.starts_with(parent_resources) {
            return Err(WorkerError::Identity);
        }
        let path = sbpl_literal(path)?;
        use std::fmt::Write as _;
        write!(&mut resource_literals, " (literal \"{path}\")")
            .map_err(|_| WorkerError::Identity)?;
    }
    Ok(format!(
        "(version 1)\n(deny default)\n(deny network*)\n(deny process-fork)\n(allow process-exec (literal \"{worker}\") (literal \"/usr/lib/dyld\"))\n(allow file-read* (subpath \"/System/Library/Frameworks/Accelerate.framework\") (subpath \"/System/Library/Frameworks/CoreFoundation.framework\") (subpath \"/System/Library/Frameworks/Foundation.framework\") (subpath \"/System/Library/Frameworks/Metal.framework\") (subpath \"/System/Library/Frameworks/MetalKit.framework\") (subpath \"/System/Library/Frameworks/Security.framework\") (literal \"/usr/lib/dyld\") (literal \"/usr/lib/libSystem.B.dylib\") (literal \"/usr/lib/libc++.1.dylib\") (literal \"/usr/lib/libobjc.A.dylib\") (literal \"{worker}\") (literal \"{model}\") (literal \"{parent_executable}\"){resource_literals})\n(allow mach-lookup (global-name \"com.apple.securityd\"))\n(allow mach-lookup (global-name \"com.apple.MTLCompilerService\"))\n"
    ))
}

fn sbpl_literal(path: &Path) -> Result<String, WorkerError> {
    let path = path.to_str().ok_or(WorkerError::Identity)?;
    if path.is_empty() || path.len() > 4096 || path.chars().any(|c| c.is_control()) {
        return Err(WorkerError::Identity);
    }
    Ok(path.replace('\\', "\\\\").replace('"', "\\\""))
}

fn set_nosigpipe(stream: &UnixStream) -> Result<(), WorkerError> {
    let value: libc::c_int = 1;
    // SAFETY: the socket option consumes one integer value and the stream remains alive.
    let result = unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            (&value as *const libc::c_int).cast(),
            std::mem::size_of_val(&value) as _,
        )
    };
    if result != 0 {
        return Err(WorkerError::Io(io::Error::last_os_error()));
    }
    Ok(())
}

fn remaining_millis(deadline: Instant, maximum: u32) -> u32 {
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .as_millis();
    u32::try_from(remaining)
        .unwrap_or(maximum)
        .min(maximum)
        .max(1)
}

fn valid_prompt_json(prompt: &str) -> bool {
    if prompt.is_empty() || prompt.len() > crate::voice::contract::MAX_HISTORY_BYTES {
        return false;
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Prompt<'a> {
        prompt_version: u8,
        system_prompt: &'a str,
        history: Vec<Pair<'a>>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Pair<'a> {
        user: &'a str,
        assistant: &'a str,
    }
    let Ok(value) = serde_json::from_str::<Prompt<'_>>(prompt) else {
        return false;
    };
    value.prompt_version == 1
        && !value.system_prompt.is_empty()
        && value.history.len() <= 20
        && value
            .history
            .iter()
            .all(|pair| !pair.user.is_empty() && !pair.assistant.is_empty())
}

fn validate_asr_result(mut result: AsrResult, mode: LanguageMode) -> Result<AsrResult, AsrError> {
    if result.text.len() > 8_000
        || result.text.chars().count() > 2_000
        || result.ranked_languages.len() > 16
    {
        return Err(AsrError::InvalidEvidence);
    }
    if result.ranked_languages.iter().any(|evidence| {
        !evidence.score.is_finite()
            || !evidence.score_gap.is_finite()
            || !(0.0..=1.0).contains(&evidence.score)
            || !(0.0..=1.0).contains(&evidence.score_gap)
    }) || result
        .no_speech_score
        .is_some_and(|score| !score.is_finite() || !(0.0..=1.0).contains(&score))
    {
        return Err(AsrError::InvalidEvidence);
    }
    if let Some(top) = result.ranked_languages.first() {
        let supported = matches!(
            (&top.language, mode),
            (
                crate::voice::contract::AsrLanguage::Tr,
                LanguageMode::Tr | LanguageMode::Auto
            ) | (
                crate::voice::contract::AsrLanguage::En,
                LanguageMode::En | LanguageMode::Auto
            )
        );
        if !supported {
            result
                .quality_flags
                .push(AsrQualityFlag::UnsupportedLanguage);
            result.eligibility = TranscriptEligibility::NeedsReview;
        }
        if mode == LanguageMode::Auto
            && !result
                .quality_flags
                .contains(&AsrQualityFlag::UncalibratedAuto)
        {
            result.quality_flags.push(AsrQualityFlag::UncalibratedAuto);
            result.eligibility = TranscriptEligibility::NeedsReview;
        }
    } else {
        return Err(AsrError::InvalidEvidence);
    }
    Ok(result)
}

fn checked_try_wait(child: &mut Child) -> Result<Option<std::process::ExitStatus>, WorkerError> {
    match child.try_wait() {
        Ok(status) => Ok(status),
        Err(_) => {
            mark_worker_cleanup_failed();
            Err(WorkerError::Cleanup)
        }
    }
}

fn terminate_child(child: &mut Child, task_identity: Option<u64>) -> Result<(), WorkerError> {
    if let Some(identity) = task_identity {
        signal_owned_child(child, identity, libc::SIGTERM)?;
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => return Ok(()),
                Ok(None) => thread::sleep(REAPER_POLL_INTERVAL),
                Err(_) => {
                    mark_worker_cleanup_failed();
                    break;
                }
            }
        }
        signal_owned_child(child, identity, libc::SIGKILL)?;
    } else if let Err(error) = child.kill() {
        if error.kind() != io::ErrorKind::InvalidInput {
            mark_worker_cleanup_failed();
            return Err(WorkerError::Cleanup);
        }
    }
    reap_child(child, REAPER_POLL_INTERVAL)
}

fn reap_child(child: &mut Child, interval: Duration) -> Result<(), WorkerError> {
    let deadline = Instant::now() + WORKER_REAP_DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
            Err(_) => mark_worker_cleanup_failed(),
        }
        if Instant::now() >= deadline {
            mark_worker_cleanup_failed();
            return Err(WorkerError::Cleanup);
        }
        thread::sleep(interval.min(REAPER_POLL_INTERVAL));
    }
}

fn signal_owned_child(
    child: &mut Child,
    task_identity: u64,
    signal: i32,
) -> Result<(), WorkerError> {
    match child.try_wait() {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(_) => mark_worker_cleanup_failed(),
    }
    let pid = i32::try_from(child.id()).map_err(|_| WorkerError::Identity)?;
    #[cfg(target_os = "macos")]
    {
        if unsafe { saturday_voice_worker_signal(pid, task_identity, signal) } != 0 {
            mark_worker_cleanup_failed();
            return Err(WorkerError::Cleanup);
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (pid, task_identity, signal);
        Err(WorkerError::Unavailable)
    }
}

fn spawned_task_identity(pid: u32) -> Result<u64, WorkerError> {
    let pid = i32::try_from(pid).map_err(|_| WorkerError::Identity)?;
    #[cfg(target_os = "macos")]
    {
        let mut identity = 0;
        if unsafe { saturday_voice_worker_task_identity(pid, &mut identity) } == 0 && identity != 0
        {
            return Ok(identity);
        }
        Err(WorkerError::Identity)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        Err(WorkerError::Unavailable)
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn saturday_voice_worker_task_identity(pid: libc::pid_t, identity_out: *mut u64)
        -> libc::c_int;
    fn saturday_voice_worker_signal(
        pid: libc::pid_t,
        identity: u64,
        signal: libc::c_int,
    ) -> libc::c_int;
    fn saturday_voice_worker_physical_footprint(
        pid: libc::pid_t,
        identity: u64,
        bytes_out: *mut u64,
    ) -> libc::c_int;
}

fn physical_footprint(pid: i32, task_identity: u64) -> Result<u64, WorkerError> {
    #[cfg(target_os = "macos")]
    {
        let mut bytes = 0;
        if unsafe { saturday_voice_worker_physical_footprint(pid, task_identity, &mut bytes) } == 0
        {
            return Ok(bytes);
        }
        Err(WorkerError::Identity)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (pid, task_identity);
        Err(WorkerError::Unavailable)
    }
}

fn mark_worker_cleanup_failed() {
    WORKER_CLEANUP_FAILED.store(true, Ordering::Release);
}

fn map_protocol_error(_error: ProtocolError) -> WorkerError {
    WorkerError::Protocol
}

fn map_conversation_error(error: WorkerError) -> ConversationFailure {
    match error {
        WorkerError::Identity | WorkerError::Handshake => ConversationFailure::WorkerIdentity,
        WorkerError::Cancelled => ConversationFailure::Cancelled,
        WorkerError::AuthorizationChanged => ConversationFailure::AssetUnavailable,
        WorkerError::ResourceLimit => ConversationFailure::Failed,
        WorkerError::Cleanup => ConversationFailure::CleanupFailed,
        WorkerError::Timeout => ConversationFailure::Timeout,
        WorkerError::Unavailable => ConversationFailure::Unavailable,
        _ => ConversationFailure::WorkerProtocol,
    }
}

fn map_asr_error(error: WorkerError) -> AsrError {
    match error {
        WorkerError::Identity | WorkerError::Handshake => AsrError::AssetUnavailable,
        WorkerError::Cancelled => AsrError::Cancelled,
        WorkerError::AuthorizationChanged => AsrError::AssetUnavailable,
        WorkerError::ResourceLimit => AsrError::WorkerFailed,
        WorkerError::Cleanup => AsrError::CleanupFailed,
        WorkerError::Timeout => AsrError::Timeout,
        WorkerError::Unavailable => AsrError::Unavailable,
        _ => AsrError::WorkerFailed,
    }
}

fn map_worker_failure(code: WorkerFailure) -> ConversationFailure {
    match code {
        WorkerFailure::RuntimeUnavailable | WorkerFailure::InvalidModel => {
            ConversationFailure::AssetUnavailable
        }
        WorkerFailure::Cancelled => ConversationFailure::Cancelled,
        WorkerFailure::Timeout => ConversationFailure::Timeout,
        WorkerFailure::CleanupFailed => ConversationFailure::CleanupFailed,
        WorkerFailure::InvalidRequest
        | WorkerFailure::InvalidAudio
        | WorkerFailure::UnsupportedLanguage => ConversationFailure::InvalidContext,
        _ => ConversationFailure::Failed,
    }
}

fn map_asr_failure(code: WorkerFailure) -> AsrError {
    match code {
        WorkerFailure::RuntimeUnavailable | WorkerFailure::InvalidModel => {
            AsrError::AssetUnavailable
        }
        WorkerFailure::InvalidAudio => AsrError::InvalidAudio,
        WorkerFailure::UnsupportedLanguage => AsrError::UnsupportedLanguage,
        WorkerFailure::Cancelled => AsrError::Cancelled,
        WorkerFailure::Timeout => AsrError::Timeout,
        WorkerFailure::CleanupFailed => AsrError::CleanupFailed,
        _ => AsrError::WorkerFailed,
    }
}

fn wire_role(role: WorkerRole) -> WireRole {
    match role {
        WorkerRole::Asr => WireRole::Asr,
        WorkerRole::Llm => WireRole::Llm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn process_incarnation_identity_is_stable_for_current_pid() {
        let pid = std::process::id();
        assert_eq!(
            spawned_task_identity(pid).unwrap(),
            spawned_task_identity(pid).unwrap()
        );
    }
    #[test]
    fn prompt_requires_exact_completed_pair_shape() {
        assert!(valid_prompt_json(
            r#"{"prompt_version":1,"system_prompt":"You are local.","history":[]}"#
        ));
        assert!(!valid_prompt_json(
            r#"{"prompt_version":1,"system_prompt":"x","history":[{"user":"a"}]}"#
        ));
        assert!(!valid_prompt_json(
            r#"{"prompt_version":1,"system_prompt":"x","history":[],"tools":[]}"#
        ));
    }

    #[test]
    fn sandbox_profile_denies_network_fork_and_unreviewed_system_paths() {
        let resource_root =
            Path::new("/Applications/Saturday.app/Contents/Resources/saturday-resources");
        let binding = resource_root.join("voice-asset-bindings.json");
        let manifest = resource_root.join("voice-runtime-manifest.json");
        let profile = build_sandbox_profile(
            Path::new("/app/saturday-resources/voice-workers/asr/worker"),
            Path::new("/private/model"),
            Path::new("/Applications/Saturday.app/Contents/MacOS/saturday"),
            resource_root,
            &[binding, manifest],
        )
        .unwrap();
        assert!(profile.contains("(deny network*)"));
        assert!(profile.contains("(deny process-fork)"));
        assert!(profile.contains("(deny default)"));
        assert!(
            profile.contains("(literal \"/Applications/Saturday.app/Contents/MacOS/saturday\")")
        );
        assert!(profile.contains("(literal \"/Applications/Saturday.app/Contents/Resources/saturday-resources/voice-asset-bindings.json\")"));
        assert!(profile.contains("(literal \"/Applications/Saturday.app/Contents/Resources/saturday-resources/voice-runtime-manifest.json\")"));
        assert!(!profile.contains(
            "(subpath \"/Applications/Saturday.app/Contents/Resources/saturday-resources\")"
        ));
        assert!(profile.contains("(subpath \"/System/Library/Frameworks/Metal.framework\")"));
        assert!(profile.contains("(allow mach-lookup (global-name \"com.apple.securityd\"))"));
        assert!(
            profile.contains("(allow mach-lookup (global-name \"com.apple.MTLCompilerService\"))")
        );
        assert_eq!(profile.matches("(allow mach-lookup").count(), 2);
        assert!(!profile.contains("(subpath \"/System/Library/PrivateFrameworks\")"));
        assert!(!profile.contains("(subpath \"/usr/lib\")"));
        assert!(!profile.contains("(allow file-write"));
        assert!(!profile.contains("(allow default)"));
    }
}
