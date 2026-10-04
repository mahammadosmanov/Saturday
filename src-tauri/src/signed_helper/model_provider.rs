use std::collections::HashSet;
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_OPERATION_IDS: usize = 256;
const MAX_OUTPUT_CHUNKS: u32 = 256;
const MAX_OUTPUT_BYTES: usize = 1_048_576;
const CANCELLATION_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelIdentity {
    pub model_sha256: String,
    pub runtime_build_id: String,
    pub template_sha256: String,
    pub context_tokens: u32,
    pub maximum_output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationRequest {
    pub operation_id: String,
    pub prompt: String,
    pub maximum_output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRequest {
    Initialize(ModelIdentity),
    Generate(GenerationRequest),
    Cancel { operation_id: String },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendEvent {
    Token {
        operation_id: String,
        sequence: u32,
        text: String,
    },
    Complete {
        operation_id: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    Cancelled {
        operation_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelEvent {
    Initialized,
    Started {
        operation_id: String,
    },
    Token {
        operation_id: String,
        sequence: u32,
        text: String,
    },
    Complete {
        operation_id: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    Cancelled {
        operation_id: String,
    },
    ShutdownAck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderState {
    Dormant,
    Ready,
    Generating,
    Cancelling,
    Closed,
    Failed,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ProviderError {
    #[error("model request is invalid")]
    InvalidRequest,
    #[error("model provider state does not allow this request")]
    InvalidState,
    #[error("model backend failed")]
    Backend,
    #[error("model protocol failed")]
    Protocol,
    #[error("model cancellation timed out")]
    CancellationTimeout,
}

pub trait ModelBackend {
    type Error;

    fn initialize(&mut self, identity: &ModelIdentity) -> Result<(), Self::Error>;
    fn start(&mut self, request: &GenerationRequest) -> Result<(), Self::Error>;
    fn cancel(&mut self, operation_id: &str) -> Result<(), Self::Error>;
    fn poll(&mut self) -> Result<Option<BackendEvent>, Self::Error>;
    fn shutdown(&mut self) -> Result<(), Self::Error>;
}

pub struct ModelProvider<B: ModelBackend> {
    backend: B,
    state: ProviderState,
    identity: Option<ModelIdentity>,
    active_operation_id: Option<String>,
    active_maximum_output_tokens: u32,
    next_sequence: u32,
    active_output_chunks: u32,
    active_output_bytes: usize,
    used_operation_ids: HashSet<String>,
    cancellation_deadline: Option<Instant>,
}

impl<B: ModelBackend> ModelProvider<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            state: ProviderState::Dormant,
            identity: None,
            active_operation_id: None,
            active_maximum_output_tokens: 0,
            next_sequence: 0,
            active_output_chunks: 0,
            active_output_bytes: 0,
            used_operation_ids: HashSet::new(),
            cancellation_deadline: None,
        }
    }

    pub fn state(&self) -> ProviderState {
        self.state
    }

    pub fn handle(&mut self, request: ModelRequest) -> Result<Option<ModelEvent>, ProviderError> {
        match request {
            ModelRequest::Initialize(identity) if self.state == ProviderState::Dormant => {
                if !valid_identity(&identity) {
                    return Err(ProviderError::InvalidRequest);
                }
                if self.backend.initialize(&identity).is_err() {
                    self.state = ProviderState::Failed;
                    return Err(ProviderError::Backend);
                }
                self.identity = Some(identity);
                self.state = ProviderState::Ready;
                Ok(Some(ModelEvent::Initialized))
            }
            ModelRequest::Generate(request) if self.state == ProviderState::Ready => {
                let identity = self.identity.as_ref().ok_or(ProviderError::InvalidState)?;
                if !valid_generation(&request, identity) {
                    return Err(ProviderError::InvalidRequest);
                }
                if self.used_operation_ids.len() >= MAX_OPERATION_IDS
                    || self.used_operation_ids.contains(&request.operation_id)
                {
                    return Err(ProviderError::InvalidRequest);
                }
                self.used_operation_ids.insert(request.operation_id.clone());
                if self.backend.start(&request).is_err() {
                    self.state = ProviderState::Failed;
                    return Err(ProviderError::Backend);
                }
                self.active_operation_id = Some(request.operation_id.clone());
                self.active_maximum_output_tokens = request.maximum_output_tokens;
                self.next_sequence = 0;
                self.active_output_chunks = 0;
                self.active_output_bytes = 0;
                self.state = ProviderState::Generating;
                Ok(Some(ModelEvent::Started {
                    operation_id: request.operation_id,
                }))
            }
            ModelRequest::Cancel { operation_id }
                if self.state == ProviderState::Generating
                    && self.active_operation_id.as_deref() == Some(operation_id.as_str()) =>
            {
                if self.backend.cancel(&operation_id).is_err() {
                    self.state = ProviderState::Failed;
                    return Err(ProviderError::Backend);
                }
                self.state = ProviderState::Cancelling;
                self.cancellation_deadline = Some(Instant::now() + CANCELLATION_TIMEOUT);
                Ok(None)
            }
            ModelRequest::Shutdown
                if !matches!(self.state, ProviderState::Dormant | ProviderState::Closed) =>
            {
                if self.backend.shutdown().is_err() {
                    self.state = ProviderState::Failed;
                    return Err(ProviderError::Backend);
                }
                self.state = ProviderState::Closed;
                Ok(Some(ModelEvent::ShutdownAck))
            }
            _ => Err(ProviderError::InvalidState),
        }
    }

    pub fn poll(&mut self) -> Result<Option<ModelEvent>, ProviderError> {
        self.poll_at(Instant::now())
    }

    fn poll_at(&mut self, now: Instant) -> Result<Option<ModelEvent>, ProviderError> {
        if !matches!(
            self.state,
            ProviderState::Generating | ProviderState::Cancelling
        ) {
            return Err(ProviderError::InvalidState);
        }
        if self.state == ProviderState::Cancelling
            && self
                .cancellation_deadline
                .is_some_and(|deadline| now >= deadline)
        {
            let _ = self.backend.shutdown();
            self.state = ProviderState::Failed;
            return Err(ProviderError::CancellationTimeout);
        }
        let event = match self.backend.poll() {
            Ok(event) => event,
            Err(_) => {
                self.state = ProviderState::Failed;
                return Err(ProviderError::Backend);
            }
        };
        let Some(event) = event else {
            return Ok(None);
        };
        let active = self
            .active_operation_id
            .as_deref()
            .ok_or(ProviderError::Protocol)?;
        let validated = match event {
            BackendEvent::Token {
                operation_id,
                sequence,
                text,
            } if self.state == ProviderState::Generating
                && operation_id == active
                && sequence == self.next_sequence
                && !text.is_empty()
                && text.len() <= 4096
                && self.active_output_chunks < MAX_OUTPUT_CHUNKS
                && self
                    .active_output_bytes
                    .checked_add(text.len())
                    .is_some_and(|bytes| bytes <= MAX_OUTPUT_BYTES) =>
            {
                self.next_sequence += 1;
                self.active_output_chunks += 1;
                self.active_output_bytes += text.len();
                Some(ModelEvent::Token {
                    operation_id,
                    sequence,
                    text,
                })
            }
            BackendEvent::Complete {
                operation_id,
                input_tokens,
                output_tokens,
            } if self.state == ProviderState::Generating
                && operation_id == active
                && output_tokens <= self.active_maximum_output_tokens
                && input_tokens
                    .checked_add(output_tokens)
                    .is_some_and(|total| {
                        total
                            <= self
                                .identity
                                .as_ref()
                                .map_or(0, |identity| identity.context_tokens)
                    }) =>
            {
                self.finish_operation();
                Some(ModelEvent::Complete {
                    operation_id,
                    input_tokens,
                    output_tokens,
                })
            }
            BackendEvent::Cancelled { operation_id }
                if self.state == ProviderState::Cancelling && operation_id == active =>
            {
                self.finish_operation();
                Some(ModelEvent::Cancelled { operation_id })
            }
            _ => {
                let _ = self.backend.shutdown();
                self.state = ProviderState::Failed;
                return Err(ProviderError::Protocol);
            }
        };
        Ok(validated)
    }

    fn finish_operation(&mut self) {
        self.active_operation_id = None;
        self.active_maximum_output_tokens = 0;
        self.next_sequence = 0;
        self.active_output_chunks = 0;
        self.active_output_bytes = 0;
        self.cancellation_deadline = None;
        self.state = ProviderState::Ready;
    }

    #[cfg(test)]
    fn backend_mut_for_test(&mut self) -> &mut B {
        &mut self.backend
    }

    #[cfg(test)]
    fn poll_at_for_test(&mut self, now: Instant) -> Result<Option<ModelEvent>, ProviderError> {
        self.poll_at(now)
    }
}

fn fixed_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn valid_identity(identity: &ModelIdentity) -> bool {
    fixed_hex(&identity.model_sha256, 64)
        && fixed_hex(&identity.runtime_build_id, 64)
        && fixed_hex(&identity.template_sha256, 64)
        && identity.context_tokens == 4096
        && identity.maximum_output_tokens == 256
}

fn valid_generation(request: &GenerationRequest, identity: &ModelIdentity) -> bool {
    fixed_hex(&request.operation_id, 32)
        && !request.prompt.is_empty()
        && request.prompt.len() <= 32768
        && (1..=identity.maximum_output_tokens).contains(&request.maximum_output_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MockFailure {
        Failed,
    }

    #[derive(Default)]
    struct MockBackend {
        initialized: usize,
        starts: Vec<String>,
        cancels: Vec<String>,
        shutdowns: usize,
        events: VecDeque<Result<BackendEvent, MockFailure>>,
        fail_initialize: bool,
    }

    impl ModelBackend for MockBackend {
        type Error = MockFailure;

        fn initialize(&mut self, _identity: &ModelIdentity) -> Result<(), Self::Error> {
            self.initialized += 1;
            if self.fail_initialize {
                Err(MockFailure::Failed)
            } else {
                Ok(())
            }
        }

        fn start(&mut self, request: &GenerationRequest) -> Result<(), Self::Error> {
            self.starts.push(request.operation_id.clone());
            Ok(())
        }

        fn cancel(&mut self, operation_id: &str) -> Result<(), Self::Error> {
            self.cancels.push(operation_id.to_owned());
            Ok(())
        }

        fn poll(&mut self) -> Result<Option<BackendEvent>, Self::Error> {
            self.events.pop_front().transpose()
        }

        fn shutdown(&mut self) -> Result<(), Self::Error> {
            self.shutdowns += 1;
            Ok(())
        }
    }

    fn identity() -> ModelIdentity {
        ModelIdentity {
            model_sha256: "a".repeat(64),
            runtime_build_id: "b".repeat(64),
            template_sha256: "c".repeat(64),
            context_tokens: 4096,
            maximum_output_tokens: 256,
        }
    }

    fn generation(operation_id: &str) -> GenerationRequest {
        GenerationRequest {
            operation_id: operation_id.to_owned(),
            prompt: "fixed local prompt".to_owned(),
            maximum_output_tokens: 128,
        }
    }

    #[test]
    fn initializes_generates_ordered_tokens_and_completes() {
        let operation_id = "d".repeat(32);
        let mut backend = MockBackend::default();
        backend.events.extend([
            Ok(BackendEvent::Token {
                operation_id: operation_id.clone(),
                sequence: 0,
                text: "local".to_owned(),
            }),
            Ok(BackendEvent::Token {
                operation_id: operation_id.clone(),
                sequence: 1,
                text: " reply".to_owned(),
            }),
            Ok(BackendEvent::Complete {
                operation_id: operation_id.clone(),
                input_tokens: 3,
                output_tokens: 2,
            }),
        ]);
        let mut provider = ModelProvider::new(backend);

        assert_eq!(
            provider
                .handle(ModelRequest::Initialize(identity()))
                .unwrap(),
            Some(ModelEvent::Initialized)
        );
        assert_eq!(provider.state(), ProviderState::Ready);
        assert_eq!(
            provider
                .handle(ModelRequest::Generate(generation(&operation_id)))
                .unwrap(),
            Some(ModelEvent::Started {
                operation_id: operation_id.clone()
            })
        );
        assert_eq!(
            provider.poll().unwrap(),
            Some(ModelEvent::Token {
                operation_id: operation_id.clone(),
                sequence: 0,
                text: "local".to_owned(),
            })
        );
        assert!(matches!(
            provider.poll().unwrap(),
            Some(ModelEvent::Token { sequence: 1, .. })
        ));
        assert_eq!(
            provider.poll().unwrap(),
            Some(ModelEvent::Complete {
                operation_id,
                input_tokens: 3,
                output_tokens: 2,
            })
        );
        assert_eq!(provider.state(), ProviderState::Ready);
    }

    #[test]
    fn rejects_invalid_identity_requests_and_parallel_work() {
        let mut provider = ModelProvider::new(MockBackend::default());
        let operation_id = "d".repeat(32);
        assert_eq!(
            provider.handle(ModelRequest::Generate(generation(&operation_id))),
            Err(ProviderError::InvalidState)
        );
        let mut invalid_identity = identity();
        invalid_identity.context_tokens = 8192;
        assert_eq!(
            provider.handle(ModelRequest::Initialize(invalid_identity)),
            Err(ProviderError::InvalidRequest)
        );
        provider
            .handle(ModelRequest::Initialize(identity()))
            .unwrap();
        assert_eq!(
            provider.handle(ModelRequest::Initialize(identity())),
            Err(ProviderError::InvalidState)
        );
        provider
            .handle(ModelRequest::Generate(generation(&operation_id)))
            .unwrap();
        assert_eq!(
            provider.handle(ModelRequest::Generate(generation(&"e".repeat(32)))),
            Err(ProviderError::InvalidState)
        );
    }

    #[test]
    fn cancellation_requires_ack_and_rejects_late_tokens() {
        let operation_id = "d".repeat(32);
        let mut backend = MockBackend::default();
        backend.events.push_back(Ok(BackendEvent::Cancelled {
            operation_id: operation_id.clone(),
        }));
        let mut provider = ModelProvider::new(backend);
        provider
            .handle(ModelRequest::Initialize(identity()))
            .unwrap();
        provider
            .handle(ModelRequest::Generate(generation(&operation_id)))
            .unwrap();
        assert_eq!(
            provider
                .handle(ModelRequest::Cancel {
                    operation_id: operation_id.clone()
                })
                .unwrap(),
            None
        );
        assert_eq!(provider.state(), ProviderState::Cancelling);
        assert_eq!(
            provider.poll().unwrap(),
            Some(ModelEvent::Cancelled {
                operation_id: operation_id.clone()
            })
        );
        assert_eq!(provider.state(), ProviderState::Ready);

        assert_eq!(
            provider.handle(ModelRequest::Generate(generation(&operation_id))),
            Err(ProviderError::InvalidRequest)
        );
        let second_operation_id = "e".repeat(32);
        provider
            .handle(ModelRequest::Generate(generation(&second_operation_id)))
            .unwrap();
        provider
            .handle(ModelRequest::Cancel {
                operation_id: second_operation_id.clone(),
            })
            .unwrap();
        provider
            .backend_mut_for_test()
            .events
            .push_back(Ok(BackendEvent::Token {
                operation_id: second_operation_id,
                sequence: 0,
                text: "late".to_owned(),
            }));
        assert_eq!(provider.poll(), Err(ProviderError::Protocol));
        assert_eq!(provider.state(), ProviderState::Failed);
    }

    #[test]
    fn validates_generation_limits_and_redacts_backend_failures() {
        let backend = MockBackend {
            fail_initialize: true,
            ..MockBackend::default()
        };
        let mut provider = ModelProvider::new(backend);
        let error = provider
            .handle(ModelRequest::Initialize(identity()))
            .unwrap_err();
        assert_eq!(error, ProviderError::Backend);
        assert_eq!(error.to_string(), "model backend failed");
        assert_eq!(provider.state(), ProviderState::Failed);

        let mut provider = ModelProvider::new(MockBackend::default());
        provider
            .handle(ModelRequest::Initialize(identity()))
            .unwrap();
        let mut request = generation(&"d".repeat(32));
        request.maximum_output_tokens = 257;
        assert_eq!(
            provider.handle(ModelRequest::Generate(request)),
            Err(ProviderError::InvalidRequest)
        );
        let mut request = generation("not-an-operation-id");
        request.prompt = "x".repeat(32769);
        assert_eq!(
            provider.handle(ModelRequest::Generate(request)),
            Err(ProviderError::InvalidRequest)
        );
    }

    #[test]
    fn shutdown_is_explicit_and_terminal() {
        let mut provider = ModelProvider::new(MockBackend::default());
        provider
            .handle(ModelRequest::Initialize(identity()))
            .unwrap();
        assert_eq!(
            provider.handle(ModelRequest::Shutdown).unwrap(),
            Some(ModelEvent::ShutdownAck)
        );
        assert_eq!(provider.state(), ProviderState::Closed);
        assert_eq!(
            provider.handle(ModelRequest::Shutdown),
            Err(ProviderError::InvalidState)
        );
    }

    #[test]
    fn cancellation_deadline_forces_backend_shutdown() {
        let operation_id = "d".repeat(32);
        let mut provider = ModelProvider::new(MockBackend::default());
        provider
            .handle(ModelRequest::Initialize(identity()))
            .unwrap();
        provider
            .handle(ModelRequest::Generate(generation(&operation_id)))
            .unwrap();
        provider
            .handle(ModelRequest::Cancel { operation_id })
            .unwrap();
        let after_deadline = provider.cancellation_deadline.unwrap() + Duration::from_millis(1);
        assert_eq!(
            provider.poll_at_for_test(after_deadline),
            Err(ProviderError::CancellationTimeout)
        );
        assert_eq!(provider.state(), ProviderState::Failed);
        assert_eq!(provider.backend_mut_for_test().shutdowns, 1);
    }

    #[test]
    fn cumulative_stream_bounds_fail_closed_and_shutdown_is_allowed_while_active() {
        let operation_id = "d".repeat(32);
        let mut backend = MockBackend::default();
        for sequence in 0..=MAX_OUTPUT_CHUNKS {
            backend.events.push_back(Ok(BackendEvent::Token {
                operation_id: operation_id.clone(),
                sequence,
                text: "x".to_owned(),
            }));
        }
        let mut provider = ModelProvider::new(backend);
        provider
            .handle(ModelRequest::Initialize(identity()))
            .unwrap();
        provider
            .handle(ModelRequest::Generate(generation(&operation_id)))
            .unwrap();
        for _ in 0..MAX_OUTPUT_CHUNKS {
            assert!(matches!(
                provider.poll(),
                Ok(Some(ModelEvent::Token { .. }))
            ));
        }
        assert_eq!(provider.poll(), Err(ProviderError::Protocol));
        assert_eq!(provider.state(), ProviderState::Failed);
        assert_eq!(
            provider.handle(ModelRequest::Shutdown).unwrap(),
            Some(ModelEvent::ShutdownAck)
        );
        assert_eq!(provider.state(), ProviderState::Closed);

        let mut provider = ModelProvider::new(MockBackend::default());
        provider
            .handle(ModelRequest::Initialize(identity()))
            .unwrap();
        provider
            .handle(ModelRequest::Generate(generation(&"f".repeat(32))))
            .unwrap();
        assert_eq!(
            provider.handle(ModelRequest::Shutdown).unwrap(),
            Some(ModelEvent::ShutdownAck)
        );
        assert_eq!(provider.state(), ProviderState::Closed);
    }
}
