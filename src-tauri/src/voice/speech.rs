use crate::platform::voice_speech::{
    copy_native_event, NativeSpeech, NativeSpeechError, NativeSpeechEvent, NativeSpeechEventKind,
    NativeVoiceRecord, MAX_DISPLAY_NAME_BYTES, MAX_LOCALE_BYTES, MAX_OWNER_ID_BYTES,
    MAX_SEGMENT_BYTES, MAX_SEGMENT_SCALARS, MAX_VOICE_ID_BYTES,
};
use crate::voice::contract::{
    CleanupCode, CleanupReceipt, EventQueueError, Language, LocalSpeechProvider, OperationId,
    SpeechAcceptance, SpeechError, SpeechSegment, TurnContext, VoiceCatalog, VoiceError,
    VoiceEventKind, VoiceId, VoiceOwner, VoiceRecord, CANCEL_ACK_DEADLINE, MAX_MODEL_OUTPUT_BYTES,
    MAX_SPOKEN_BYTES, MAX_SPOKEN_SCALARS, MAX_SUPPORTED_VOICES,
};
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const NATIVE_EVENT_CAPACITY: usize = 32;
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(5);
const MAX_NATIVE_UTTERANCES: usize = 3;

pub trait SpeechPlatform: Send + Sync + 'static {
    fn enumerate_voices(&self) -> Result<(Vec<NativeVoiceRecord>, u64), NativeSpeechError>;
    fn start(
        &self,
        context: &TurnContext,
        segment: &SpeechSegment,
        locale: &str,
        utterance_id: u64,
    ) -> Result<(), NativeSpeechError>;
    fn available_credits(&self) -> Result<u8, NativeSpeechError>;
    fn cancel(&self, owner: &VoiceOwner) -> Result<(), NativeSpeechError>;
    fn is_stopped(&self) -> Result<bool, NativeSpeechError>;
    fn destroy(&self) -> Result<(), NativeSpeechError>;
    fn normalize_nfc(&self, text: &str) -> Result<String, NativeSpeechError>;
}

impl SpeechPlatform for NativeSpeech {
    fn enumerate_voices(&self) -> Result<(Vec<NativeVoiceRecord>, u64), NativeSpeechError> {
        NativeSpeech::enumerate_voices()
    }

    fn start(
        &self,
        context: &TurnContext,
        segment: &SpeechSegment,
        locale: &str,
        utterance_id: u64,
    ) -> Result<(), NativeSpeechError> {
        NativeSpeech::start(self, context, segment, locale, utterance_id)
    }

    fn available_credits(&self) -> Result<u8, NativeSpeechError> {
        NativeSpeech::available_credits(self)
    }

    fn cancel(&self, owner: &VoiceOwner) -> Result<(), NativeSpeechError> {
        NativeSpeech::cancel(self, owner)
    }

    fn is_stopped(&self) -> Result<bool, NativeSpeechError> {
        NativeSpeech::is_stopped(self)
    }

    fn destroy(&self) -> Result<(), NativeSpeechError> {
        NativeSpeech::destroy(self)
    }

    fn normalize_nfc(&self, text: &str) -> Result<String, NativeSpeechError> {
        NativeSpeech::normalize_nfc(text)
    }
}

pub fn speech_event_channel() -> (SyncSender<NativeSpeechEvent>, Receiver<NativeSpeechEvent>) {
    mpsc::sync_channel(NATIVE_EVENT_CAPACITY)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechQueueAdmission {
    Start,
    Queued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechQueueError {
    Full,
    WrongOwner,
    DuplicateOperation,
    InvalidIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SpeechOperationKey {
    session_id: String,
    turn_id: String,
    generation: u64,
    operation_id: u64,
    utterance_id: u64,
}

impl SpeechOperationKey {
    fn new(owner: &VoiceOwner, operation_id: OperationId, utterance_id: u64) -> Self {
        Self {
            session_id: owner.session_id.clone(),
            turn_id: owner.turn_id.clone(),
            generation: owner.generation,
            operation_id: operation_id.0,
            utterance_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SpeechOperation {
    owner: VoiceOwner,
    operation_id: OperationId,
    utterance_id: u64,
    started: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeechTransitionKind {
    Started,
    Finished,
    Cancelled,
    Failed,
    RouteLost,
}

#[derive(Debug, Clone)]
pub struct SpeechQueueTransition {
    key: SpeechOperationKey,
    kind: SpeechTransitionKind,
    error_code: u32,
}

#[derive(Debug, Default)]
pub struct BoundedSpeechQueue {
    operations: VecDeque<SpeechOperation>,
}

impl BoundedSpeechQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn admit(
        &mut self,
        owner: VoiceOwner,
        operation_id: OperationId,
        utterance_id: u64,
    ) -> Result<SpeechQueueAdmission, SpeechQueueError> {
        if owner.session_id.is_empty()
            || owner.turn_id.is_empty()
            || owner.generation == 0
            || operation_id.0 == 0
            || utterance_id == 0
        {
            return Err(SpeechQueueError::InvalidIdentity);
        }
        if self.operations.len() >= MAX_NATIVE_UTTERANCES {
            return Err(SpeechQueueError::Full);
        }
        if let Some(current) = self.operations.front() {
            if current.owner != owner {
                return Err(SpeechQueueError::WrongOwner);
            }
        }
        if self.operations.iter().any(|operation| {
            operation.owner == owner
                && operation.operation_id == operation_id
                && operation.utterance_id == utterance_id
        }) {
            return Err(SpeechQueueError::DuplicateOperation);
        }
        let admission = if self.operations.is_empty() {
            SpeechQueueAdmission::Start
        } else {
            SpeechQueueAdmission::Queued
        };
        self.operations.push_back(SpeechOperation {
            owner,
            operation_id,
            utterance_id,
            started: false,
        });
        Ok(admission)
    }

    pub fn on_native_event(&mut self, event: &NativeSpeechEvent) -> Option<SpeechQueueTransition> {
        let index = self.operations.iter().position(|operation| {
            operation.owner == event.owner
                && operation.operation_id.0 == event.operation_id
                && operation.utterance_id == event.utterance_id
        })?;
        let key = SpeechOperationKey::new(
            &event.owner,
            OperationId(event.operation_id),
            event.utterance_id,
        );
        let transition_kind = match event.kind {
            NativeSpeechEventKind::DidStart => {
                if index != 0 || self.operations[index].started {
                    return None;
                }
                self.operations[index].started = true;
                SpeechTransitionKind::Started
            }
            NativeSpeechEventKind::DidFinish => {
                if index != 0 || !self.operations[index].started {
                    return None;
                }
                self.operations.remove(index);
                SpeechTransitionKind::Finished
            }
            NativeSpeechEventKind::Cancelled => {
                self.operations.remove(index);
                SpeechTransitionKind::Cancelled
            }
            NativeSpeechEventKind::Failed => {
                self.operations.remove(index);
                SpeechTransitionKind::Failed
            }
            NativeSpeechEventKind::RouteLost => {
                self.operations.remove(index);
                SpeechTransitionKind::RouteLost
            }
        };
        Some(SpeechQueueTransition {
            key,
            kind: transition_kind,
            error_code: event.error_code,
        })
    }

    pub fn cancel_owner(&mut self, owner: &VoiceOwner) -> Vec<SpeechOperation> {
        let mut removed = Vec::new();
        let mut retained = VecDeque::with_capacity(self.operations.len());
        while let Some(operation) = self.operations.pop_front() {
            if operation.owner == *owner {
                removed.push(operation);
            } else {
                retained.push_back(operation);
            }
        }
        self.operations = retained;
        removed
    }

    fn remove_key(&mut self, key: &SpeechOperationKey) -> bool {
        if let Some(index) = self.operations.iter().position(|operation| {
            operation.owner.session_id == key.session_id
                && operation.owner.turn_id == key.turn_id
                && operation.owner.generation == key.generation
                && operation.operation_id.0 == key.operation_id
                && operation.utterance_id == key.utterance_id
        }) {
            self.operations.remove(index);
            true
        } else {
            false
        }
    }

    pub fn is_speaking(&self) -> bool {
        self.operations
            .front()
            .is_some_and(|operation| operation.started)
    }

    pub fn queued_len(&self) -> usize {
        self.operations.len()
    }

    fn current_owner(&self) -> Option<VoiceOwner> {
        self.operations
            .front()
            .map(|operation| operation.owner.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechProjectionDisclosure {
    NonSpokenContentOmitted,
    SpeechLimitExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlainSpeechProjection {
    pub display_text: String,
    pub segments: Vec<String>,
    pub disclosures: Vec<SpeechProjectionDisclosure>,
}

impl PlainSpeechProjection {
    pub fn from_answer(answer: &str) -> Result<Self, SpeechError> {
        if answer.len() > MAX_MODEL_OUTPUT_BYTES {
            return Err(SpeechError::TooLong);
        }
        let normalized = NativeSpeech::normalize_nfc(answer).map_err(map_native_error)?;
        let (without_nonspoken, omitted) = strip_nonspoken_content(&normalized);
        let plain_text = collapse_whitespace(&without_nonspoken);
        let mut disclosures = Vec::with_capacity(2);
        if omitted {
            disclosures.push(SpeechProjectionDisclosure::NonSpokenContentOmitted);
        }
        let spoken_scalars = plain_text.chars().count();
        if spoken_scalars > MAX_SPOKEN_SCALARS || plain_text.len() > MAX_SPOKEN_BYTES {
            disclosures.push(SpeechProjectionDisclosure::SpeechLimitExceeded);
            return Ok(Self {
                display_text: answer.to_owned(),
                segments: Vec::new(),
                disclosures,
            });
        }
        let Some(segments) = split_speech_segments(&plain_text) else {
            disclosures.push(SpeechProjectionDisclosure::SpeechLimitExceeded);
            return Ok(Self {
                display_text: answer.to_owned(),
                segments: Vec::new(),
                disclosures,
            });
        };
        Ok(Self {
            display_text: answer.to_owned(),
            segments,
            disclosures,
        })
    }
}

pub fn voice_catalog_from_native(
    records: Vec<NativeVoiceRecord>,
    revision: u64,
) -> Result<VoiceCatalog, SpeechError> {
    let mut turkish = Vec::new();
    let mut english = Vec::new();
    let mut identifiers =
        HashMap::<String, ()>::with_capacity(records.len().min(MAX_SUPPORTED_VOICES));
    let mut supported_count = 0;
    for record in records {
        if !locale_matches_language(&record.locale, record.language) {
            continue;
        }
        supported_count += 1;
        if supported_count > MAX_SUPPORTED_VOICES {
            return Err(SpeechError::Unavailable);
        }
        if record.id.is_empty()
            || record.id.len() > MAX_VOICE_ID_BYTES
            || record.display_name.is_empty()
            || record.display_name.len() > MAX_DISPLAY_NAME_BYTES
            || record.locale.is_empty()
            || record.locale.len() > MAX_LOCALE_BYTES
        {
            return Err(SpeechError::Failed);
        }
        if identifiers.insert(record.id.clone(), ()).is_some() {
            return Err(SpeechError::Failed);
        }
        let voice = VoiceRecord {
            id: VoiceId(record.id),
            display_name: record.display_name,
            locale: record.locale,
            language: record.language,
        };
        match voice.language {
            Language::Tr => turkish.push(voice),
            Language::En => english.push(voice),
        }
    }
    turkish.sort_by(|left, right| left.id.0.cmp(&right.id.0));
    english.sort_by(|left, right| left.id.0.cmp(&right.id.0));
    Ok(VoiceCatalog {
        revision,
        turkish,
        english,
    })
}

pub fn resolve_voice<'a>(
    catalog: &'a VoiceCatalog,
    segment: &SpeechSegment,
) -> Result<&'a VoiceRecord, SpeechError> {
    let voice = catalog
        .turkish
        .iter()
        .chain(catalog.english.iter())
        .find(|voice| voice.id == segment.voice_id)
        .ok_or(SpeechError::VoiceUnavailable)?;
    if voice.language != segment.language
        || !locale_matches_language(&voice.locale, segment.language)
    {
        return Err(SpeechError::WrongLanguage);
    }
    Ok(voice)
}

struct ProviderState {
    queue: BoundedSpeechQueue,
    contexts: HashMap<SpeechOperationKey, TurnContext>,
    route_lost_owner: Option<VoiceOwner>,
}

impl Default for ProviderState {
    fn default() -> Self {
        Self {
            queue: BoundedSpeechQueue::new(),
            contexts: HashMap::with_capacity(MAX_NATIVE_UTTERANCES),
            route_lost_owner: None,
        }
    }
}

fn take_owner_operations(
    state: &mut ProviderState,
    owner: &VoiceOwner,
) -> Vec<(SpeechOperation, TurnContext)> {
    let removed = state.queue.cancel_owner(owner);
    let mut operations = Vec::with_capacity(removed.len());
    for operation in removed {
        let key = SpeechOperationKey::new(
            &operation.owner,
            operation.operation_id,
            operation.utterance_id,
        );
        if let Some(context) = state.contexts.remove(&key) {
            operations.push((operation, context));
        }
    }
    operations
}

struct CallbackContext {
    sender: SyncSender<NativeSpeechEvent>,
    overflowed: Arc<AtomicBool>,
    route_lost: Arc<AtomicBool>,
}

pub struct VoiceSpeechProvider<P: SpeechPlatform> {
    platform: Arc<P>,
    _event_sender: SyncSender<NativeSpeechEvent>,
    state: Arc<Mutex<ProviderState>>,
    shutdown: Arc<AtomicBool>,
    route_lost: Arc<AtomicBool>,
    permanently_blocked: Arc<AtomicBool>,
    next_utterance_id: AtomicU64,
    worker: Option<JoinHandle<()>>,
    callback_context: Option<Box<CallbackContext>>,
    unloaded: bool,
}

impl<P: SpeechPlatform> VoiceSpeechProvider<P> {
    pub fn from_platform(
        platform: Arc<P>,
        event_sender: SyncSender<NativeSpeechEvent>,
        event_receiver: Receiver<NativeSpeechEvent>,
    ) -> Result<Self, SpeechError> {
        Self::build(
            platform,
            event_sender,
            event_receiver,
            None,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        )
        .map_err(|(error, _)| error)
    }

    fn build(
        platform: Arc<P>,
        event_sender: SyncSender<NativeSpeechEvent>,
        event_receiver: Receiver<NativeSpeechEvent>,
        callback_context: Option<Box<CallbackContext>>,
        overflowed: Arc<AtomicBool>,
        route_lost: Arc<AtomicBool>,
    ) -> Result<Self, (SpeechError, Option<Box<CallbackContext>>)> {
        let state = Arc::new(Mutex::new(ProviderState::default()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let permanently_blocked = Arc::new(AtomicBool::new(false));
        let worker = {
            let platform = Arc::clone(&platform);
            let state = Arc::clone(&state);
            let shutdown = Arc::clone(&shutdown);
            let overflowed = Arc::clone(&overflowed);
            let route_lost = Arc::clone(&route_lost);
            let permanently_blocked = Arc::clone(&permanently_blocked);
            thread::Builder::new()
                .name("saturday-speech-events".to_owned())
                .spawn(move || {
                    speech_event_worker(
                        platform,
                        state,
                        event_receiver,
                        shutdown,
                        overflowed,
                        route_lost,
                        permanently_blocked,
                    )
                })
        };
        let worker = match worker {
            Ok(worker) => worker,
            Err(_) => return Err((SpeechError::Unavailable, callback_context)),
        };
        Ok(Self {
            platform,
            _event_sender: event_sender,
            state,
            shutdown,
            route_lost,
            permanently_blocked,
            next_utterance_id: AtomicU64::new(1),
            worker: Some(worker),
            callback_context,
            unloaded: false,
        })
    }

    fn available_credits_inner(&self) -> u8 {
        if self.unloaded
            || self.permanently_blocked.load(Ordering::Acquire)
            || self.route_lost.load(Ordering::Acquire)
        {
            return 0;
        }
        self.platform
            .available_credits()
            .ok()
            .filter(|credits| *credits <= MAX_NATIVE_UTTERANCES as u8)
            .unwrap_or(0)
    }

    fn send_terminal(
        context: &TurnContext,
        receipt: CleanupReceipt,
    ) -> Result<(), EventQueueError> {
        context.try_send_event(VoiceEventKind::Terminal { receipt })
    }

    pub fn queue_len(&self) -> usize {
        lock_unpoisoned(&self.state).queue.queued_len()
    }
}

pub type SystemSpeechProvider = VoiceSpeechProvider<NativeSpeech>;

impl VoiceSpeechProvider<NativeSpeech> {
    pub fn system() -> Result<Self, SpeechError> {
        let (event_sender, event_receiver) = speech_event_channel();
        let overflowed = Arc::new(AtomicBool::new(false));
        let route_lost = Arc::new(AtomicBool::new(false));
        let callback_context = Box::new(CallbackContext {
            sender: event_sender.clone(),
            overflowed: Arc::clone(&overflowed),
            route_lost: Arc::clone(&route_lost),
        });
        let user_data = (&*callback_context as *const CallbackContext)
            .cast_mut()
            .cast::<c_void>();
        // SAFETY: `callback_context` is transferred into the provider on success and
        // retained until native handle destruction; failure destroys the handle before
        // dropping the context (or leaks it if native destruction cannot be confirmed).
        let platform = match unsafe { NativeSpeech::create(native_speech_callback, user_data) } {
            Ok(platform) => Arc::new(platform),
            Err(error) => return Err(map_native_error(error)),
        };
        let result = Self::build(
            Arc::clone(&platform),
            event_sender,
            event_receiver,
            Some(callback_context),
            overflowed,
            route_lost,
        );
        match result {
            Ok(provider) => Ok(provider),
            Err((error, callback_context)) => {
                if platform.destroy().is_err() {
                    if let Some(callback_context) = callback_context {
                        let _ = Box::leak(callback_context);
                    }
                }
                Err(error)
            }
        }
    }
}

impl<P: SpeechPlatform> LocalSpeechProvider for VoiceSpeechProvider<P> {
    fn enumerate_voices(&self) -> Result<VoiceCatalog, SpeechError> {
        let (records, revision) = self.platform.enumerate_voices().map_err(map_native_error)?;
        voice_catalog_from_native(records, revision)
    }

    fn available_credits(&self) -> u8 {
        self.available_credits_inner()
    }

    fn speak(
        &mut self,
        context: &TurnContext,
        mut segment: SpeechSegment,
    ) -> Result<SpeechAcceptance, SpeechError> {
        if self.unloaded || self.permanently_blocked.load(Ordering::Acquire) {
            return Err(SpeechError::Unavailable);
        }
        if context.is_cancelled_or_expired() {
            return Err(if context.cancel.is_cancelled() {
                SpeechError::Cancelled
            } else {
                SpeechError::Timeout
            });
        }
        if context.owner.session_id.is_empty()
            || context.owner.turn_id.is_empty()
            || context.owner.generation == 0
            || context.owner.session_id.len() > MAX_OWNER_ID_BYTES
            || context.owner.turn_id.len() > MAX_OWNER_ID_BYTES
            || context.operation_id.0 == 0
        {
            return Err(SpeechError::Failed);
        }
        segment.text = self
            .platform
            .normalize_nfc(&segment.text)
            .map_err(map_native_error)?;
        validate_plain_segment(&segment.text)?;
        if segment.text.len() > MAX_SEGMENT_BYTES
            || segment.text.chars().count() > MAX_SEGMENT_SCALARS
        {
            return Err(SpeechError::TooLong);
        }
        let catalog = self.enumerate_voices()?;
        let voice = resolve_voice(&catalog, &segment)?;
        if self.route_lost.load(Ordering::Acquire) {
            let previous_owner = lock_unpoisoned(&self.state).route_lost_owner.clone();
            if previous_owner
                .as_ref()
                .is_none_or(|previous| previous == &context.owner)
            {
                return Err(SpeechError::Unavailable);
            }
            if !wait_for_stop(&*self.platform, Instant::now() + CANCEL_ACK_DEADLINE) {
                self.permanently_blocked.store(true, Ordering::Release);
                return Err(SpeechError::CleanupFailed);
            }
            self.route_lost.store(false, Ordering::Release);
            lock_unpoisoned(&self.state).route_lost_owner = None;
        }
        if context.is_cancelled_or_expired() {
            return Err(if context.cancel.is_cancelled() {
                SpeechError::Cancelled
            } else {
                SpeechError::Timeout
            });
        }
        if self.available_credits_inner() == 0 {
            return Err(SpeechError::Backpressure);
        }
        let utterance_id = self
            .next_utterance_id
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .map_err(|_| SpeechError::Failed)?;
        if utterance_id == 0 {
            return Err(SpeechError::Failed);
        }
        let key = SpeechOperationKey::new(&context.owner, context.operation_id, utterance_id);
        {
            let mut state = lock_unpoisoned(&self.state);
            state
                .queue
                .admit(context.owner.clone(), context.operation_id, utterance_id)
                .map_err(|error| match error {
                    SpeechQueueError::Full => SpeechError::Backpressure,
                    SpeechQueueError::WrongOwner
                    | SpeechQueueError::DuplicateOperation
                    | SpeechQueueError::InvalidIdentity => SpeechError::Failed,
                })?;
            state.contexts.insert(key.clone(), context.clone());
        }
        let native_result = self
            .platform
            .start(context, &segment, &voice.locale, utterance_id);
        if let Err(error) = native_result {
            let mut state = lock_unpoisoned(&self.state);
            state.queue.remove_key(&key);
            state.contexts.remove(&key);
            return Err(map_native_error(error));
        }
        Ok(SpeechAcceptance::Accepted)
    }

    fn cancel(&mut self, context: &TurnContext) -> Result<CleanupReceipt, SpeechError> {
        context.cancel.cancel();
        let deadline = Instant::now() + CANCEL_ACK_DEADLINE;
        let removed = take_owner_operations(&mut lock_unpoisoned(&self.state), &context.owner);
        if removed.is_empty() {
            return Ok(CleanupReceipt::verified(CleanupCode::NoResourcesOwned));
        }
        let _ = self.platform.cancel(&context.owner);
        let stopped = wait_for_stop(&*self.platform, deadline);
        if !stopped {
            self.permanently_blocked.store(true, Ordering::Release);
        }
        let receipt = if stopped {
            CleanupReceipt::verified(CleanupCode::SpeechStopped)
        } else {
            CleanupReceipt::failed()
        };
        let mut terminal_sent = true;
        for (_operation, operation_context) in removed {
            if !stopped {
                let _ = operation_context.try_send_event(VoiceEventKind::Failure {
                    code: VoiceError::ProviderFailed,
                });
            }
            if Self::send_terminal(&operation_context, receipt).is_err() {
                terminal_sent = false;
            }
            let _ = operation_context.try_send_event(VoiceEventKind::OutputDrained);
        }
        if !stopped || !terminal_sent {
            return Err(SpeechError::CleanupFailed);
        }
        Ok(receipt)
    }

    fn unload(&mut self) -> Result<CleanupReceipt, SpeechError> {
        if self.unloaded {
            return Ok(CleanupReceipt::verified(CleanupCode::NoResourcesOwned));
        }
        let owner = lock_unpoisoned(&self.state).queue.current_owner();
        let deadline = Instant::now() + CANCEL_ACK_DEADLINE;
        let removed = if let Some(owner) = owner.as_ref() {
            take_owner_operations(&mut lock_unpoisoned(&self.state), owner)
        } else {
            Vec::new()
        };
        if let Some(owner) = owner.as_ref() {
            let _ = self.platform.cancel(owner);
        }
        let stopped = wait_for_stop(&*self.platform, deadline);
        if !stopped {
            self.permanently_blocked.store(true, Ordering::Release);
        }
        let receipt = if stopped {
            CleanupReceipt::verified(CleanupCode::SpeechStopped)
        } else {
            CleanupReceipt::failed()
        };
        let mut terminal_sent = true;
        for (_operation, operation_context) in removed {
            if !stopped {
                let _ = operation_context.try_send_event(VoiceEventKind::Failure {
                    code: VoiceError::ProviderFailed,
                });
            }
            if Self::send_terminal(&operation_context, receipt).is_err() {
                terminal_sent = false;
            }
            let _ = operation_context.try_send_event(VoiceEventKind::OutputDrained);
        }
        if !stopped {
            return Err(SpeechError::CleanupFailed);
        }
        self.shutdown.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                self.permanently_blocked.store(true, Ordering::Release);
                return Err(SpeechError::CleanupFailed);
            }
        }
        self.platform.destroy().map_err(map_native_error)?;
        self.callback_context.take();
        self.unloaded = true;
        if !terminal_sent {
            return Err(SpeechError::CleanupFailed);
        }
        Ok(receipt)
    }
}

impl<P: SpeechPlatform> Drop for VoiceSpeechProvider<P> {
    fn drop(&mut self) {
        if self.unloaded {
            return;
        }
        if self.unload().is_err() {
            self.shutdown.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            if let Some(callback_context) = self.callback_context.take() {
                let _ = Box::leak(callback_context);
            }
        }
    }
}

fn speech_event_worker<P: SpeechPlatform>(
    platform: Arc<P>,
    state: Arc<Mutex<ProviderState>>,
    receiver: Receiver<NativeSpeechEvent>,
    shutdown: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    route_lost: Arc<AtomicBool>,
    permanently_blocked: Arc<AtomicBool>,
) {
    loop {
        if overflowed.swap(false, Ordering::AcqRel) {
            permanently_blocked.store(true, Ordering::Release);
            let owner = lock_unpoisoned(&state).queue.current_owner();
            if let Some(owner) = owner {
                stop_and_fail_owner(
                    &*platform,
                    &state,
                    &owner,
                    &permanently_blocked,
                    VoiceError::ProtocolError,
                );
            }
        }
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        match receiver.recv_timeout(EVENT_POLL_INTERVAL) {
            Ok(event) => {
                process_native_event(&*platform, &state, event, &route_lost, &permanently_blocked)
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn stop_and_fail_owner<P: SpeechPlatform>(
    platform: &P,
    state: &Mutex<ProviderState>,
    owner: &VoiceOwner,
    permanently_blocked: &AtomicBool,
    code: VoiceError,
) {
    let removed = take_owner_operations(&mut lock_unpoisoned(state), owner);
    let _ = platform.cancel(owner);
    let stopped = wait_for_stop(platform, Instant::now() + CANCEL_ACK_DEADLINE);
    if !stopped {
        permanently_blocked.store(true, Ordering::Release);
    }
    let receipt = if stopped {
        CleanupReceipt::verified(CleanupCode::SpeechStopped)
    } else {
        CleanupReceipt::failed()
    };
    for (_operation, context) in removed {
        let _ = context.try_send_event(VoiceEventKind::Failure { code });
        let _ = VoiceSpeechProvider::<P>::send_terminal(&context, receipt);
        let _ = context.try_send_event(VoiceEventKind::OutputDrained);
    }
}

fn post_terminal(context: &TurnContext, receipt: CleanupReceipt, blocked: &AtomicBool) {
    let terminal_result = context.try_send_event(VoiceEventKind::Terminal { receipt });
    let drained_result = context.try_send_event(VoiceEventKind::OutputDrained);
    if terminal_result.is_err() || drained_result.is_err() {
        blocked.store(true, Ordering::Release);
    }
}

fn process_native_event<P: SpeechPlatform>(
    platform: &P,
    state: &Mutex<ProviderState>,
    event: NativeSpeechEvent,
    route_lost: &AtomicBool,
    permanently_blocked: &AtomicBool,
) {
    let transition = {
        let mut state = lock_unpoisoned(state);
        let Some(transition) = state.queue.on_native_event(&event) else {
            return;
        };
        if transition.kind == SpeechTransitionKind::RouteLost {
            state.route_lost_owner = Some(event.owner.clone());
            route_lost.store(true, Ordering::Release);
        }
        let context = if transition.kind == SpeechTransitionKind::Started {
            state.contexts.get(&transition.key).cloned()
        } else {
            state.contexts.remove(&transition.key)
        };
        (transition, context)
    };
    let (transition, context) = transition;
    let Some(context) = context else {
        permanently_blocked.store(true, Ordering::Release);
        let _ = platform.cancel(&event.owner);
        let _ = wait_for_stop(platform, Instant::now() + CANCEL_ACK_DEADLINE);
        return;
    };
    match transition.kind {
        SpeechTransitionKind::Started => {
            if context
                .try_send_event(VoiceEventKind::OutputStarted)
                .is_err()
            {
                stop_and_fail_owner(
                    platform,
                    state,
                    &context.owner,
                    permanently_blocked,
                    VoiceError::ProtocolError,
                );
            }
        }
        SpeechTransitionKind::Finished => {
            post_terminal(
                &context,
                CleanupReceipt::verified(CleanupCode::SpeechStopped),
                permanently_blocked,
            );
        }
        SpeechTransitionKind::Cancelled => {
            let stopped = wait_for_stop(platform, Instant::now() + CANCEL_ACK_DEADLINE);
            if !stopped {
                permanently_blocked.store(true, Ordering::Release);
            }
            let receipt = if stopped {
                CleanupReceipt::verified(CleanupCode::SpeechStopped)
            } else {
                CleanupReceipt::failed()
            };
            post_terminal(&context, receipt, permanently_blocked);
        }
        SpeechTransitionKind::Failed => {
            if context
                .try_send_event(VoiceEventKind::Failure {
                    code: if transition.error_code == 0 {
                        VoiceError::ProviderFailed
                    } else {
                        VoiceError::Unavailable
                    },
                })
                .is_err()
            {
                permanently_blocked.store(true, Ordering::Release);
            }
            post_terminal(
                &context,
                CleanupReceipt::verified(CleanupCode::Verified),
                permanently_blocked,
            );
        }
        SpeechTransitionKind::RouteLost => {
            let stopped = wait_for_stop(platform, Instant::now() + CANCEL_ACK_DEADLINE);
            if !stopped {
                permanently_blocked.store(true, Ordering::Release);
            }
            let receipt = if stopped {
                CleanupReceipt::verified(CleanupCode::SpeechStopped)
            } else {
                CleanupReceipt::failed()
            };
            if context
                .try_send_event(VoiceEventKind::Failure {
                    code: VoiceError::Unavailable,
                })
                .is_err()
            {
                permanently_blocked.store(true, Ordering::Release);
            }
            post_terminal(&context, receipt, permanently_blocked);
        }
    }
}

unsafe extern "C" fn native_speech_callback(user_data: *mut c_void, event: *const c_void) {
    if user_data.is_null() {
        return;
    }
    // SAFETY: NativeSpeech::create's caller contract keeps this boxed context alive until
    // handle destruction and native callback dispatch has quiesced. The native callback
    // supplies a readable event valid for the duration of this callback invocation.
    let callback = unsafe { &*(user_data.cast::<CallbackContext>()) };
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `event` is the native callback's readable event allocation, valid during
        // this invocation; copy_native_event copies all data before returning.
        unsafe { copy_native_event(event) }
    }));
    let Ok(Ok(event)) = result else {
        callback.overflowed.store(true, Ordering::Release);
        return;
    };
    if event.kind == NativeSpeechEventKind::RouteLost {
        callback.route_lost.store(true, Ordering::Release);
    }
    match callback.sender.try_send(event) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
            callback.overflowed.store(true, Ordering::Release);
        }
    }
}

fn wait_for_stop<P: SpeechPlatform + ?Sized>(platform: &P, deadline: Instant) -> bool {
    loop {
        match platform.is_stopped() {
            Ok(true) => return true,
            Ok(false) => {}
            Err(_) => return false,
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(STOP_POLL_INTERVAL);
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn map_native_error(error: NativeSpeechError) -> SpeechError {
    match error {
        NativeSpeechError::Unavailable | NativeSpeechError::BufferTooSmall => {
            SpeechError::Unavailable
        }
        NativeSpeechError::QueueFull => SpeechError::Backpressure,
        NativeSpeechError::VoiceUnavailable => SpeechError::VoiceUnavailable,
        NativeSpeechError::WrongLocale => SpeechError::WrongLanguage,
        NativeSpeechError::RouteLost => SpeechError::Unavailable,
        NativeSpeechError::StopUnverified => SpeechError::CleanupFailed,
        NativeSpeechError::InvalidArgument | NativeSpeechError::Failed => SpeechError::Failed,
    }
}

fn locale_matches_language(locale: &str, language: Language) -> bool {
    let Some((primary, _)) = locale.split_once('-') else {
        return false;
    };
    match language {
        Language::Tr => primary.eq_ignore_ascii_case("tr"),
        Language::En => primary.eq_ignore_ascii_case("en"),
    }
}

fn validate_plain_segment(text: &str) -> Result<(), SpeechError> {
    if text.is_empty()
        || text.chars().any(is_forbidden_speech_scalar)
        || contains_nonspoken_marker(text)
    {
        return Err(SpeechError::Failed);
    }
    Ok(())
}

fn contains_nonspoken_marker(text: &str) -> bool {
    let mut previous = None;
    for (index, ch) in text.char_indices() {
        let rest = &text[index..];
        if rest.starts_with('`') || rest.starts_with('<') && tag_end(rest).is_some() {
            return true;
        }
        if previous.is_none_or(|before: char| before.is_whitespace() || "([{<\"'".contains(before))
            && is_url_token_start(rest)
        {
            return true;
        }
        previous = Some(ch);
    }
    false
}

fn strip_nonspoken_content(text: &str) -> (String, bool) {
    let mut output = String::with_capacity(text.len());
    let mut omitted = false;
    let mut index = 0;
    let mut previous = None;
    while index < text.len() {
        let rest = &text[index..];
        if rest.starts_with("```") {
            omitted = true;
            let after_open = index + 3;
            let Some(close) = text[after_open..].find("```") else {
                break;
            };
            index = after_open + close + 3;
            previous = Some('`');
            continue;
        }
        if rest.starts_with('`') {
            omitted = true;
            let after_open = index + 1;
            let Some(close) = text[after_open..].find('`') else {
                break;
            };
            index = after_open + close + 1;
            previous = Some('`');
            continue;
        }
        if rest.starts_with('[') {
            if let Some((label_start, label_end, link_end)) = markdown_link_bounds(rest) {
                output.push_str(&rest[label_start..label_end]);
                index += link_end;
                previous = Some(')');
                omitted = true;
                continue;
            }
        }
        if rest.starts_with("<!--") {
            omitted = true;
            let Some(close) = rest.find("-->") else {
                break;
            };
            index += close + 3;
            previous = Some('>');
            continue;
        }
        if rest.starts_with('<') {
            if let Some((tag_length, tag_name)) = tag_end(rest) {
                omitted = true;
                if tag_name.eq_ignore_ascii_case("script") || tag_name.eq_ignore_ascii_case("style")
                {
                    let closing = format!("</{tag_name}>");
                    let after_open = index + tag_length;
                    let lower = text[after_open..].to_ascii_lowercase();
                    let Some(close) = lower.find(&closing.to_ascii_lowercase()) else {
                        break;
                    };
                    index = after_open + close + closing.len();
                } else {
                    index += tag_length;
                }
                previous = Some('>');
                continue;
            }
        }
        if previous.is_none_or(|before: char| before.is_whitespace() || "([{<\"'".contains(before))
            && is_url_token_start(rest)
        {
            let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let token = &rest[..token_end];
            let core_end = token.trim_end_matches(is_url_trailing_punctuation).len();
            output.push_str(&token[core_end..]);
            index += token_end;
            previous = token.chars().next_back().or(previous);
            omitted = true;
            continue;
        }
        let ch = text[index..].chars().next().expect("valid UTF-8 boundary");
        if is_forbidden_speech_scalar(ch) {
            omitted = true;
        } else {
            output.push(ch);
        }
        index += ch.len_utf8();
        previous = Some(ch);
    }
    (strip_markdown_emphasis(&output), omitted)
}

fn markdown_link_bounds(text: &str) -> Option<(usize, usize, usize)> {
    let close_label = text.find("](")?;
    let close_link = text[close_label + 2..].find(')')? + close_label + 2;
    Some((1, close_label, close_link + 1))
}

fn tag_end(text: &str) -> Option<(usize, String)> {
    let rest = text.strip_prefix('<')?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let rest = rest.strip_prefix('!').unwrap_or(rest);
    let rest = rest.strip_prefix('?').unwrap_or(rest);
    let name_length = rest
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == ':')
        .last()
        .map(|(index, ch)| index + ch.len_utf8())?;
    if !rest[..name_length]
        .chars()
        .any(|ch| ch.is_ascii_alphabetic())
    {
        return None;
    }
    let close = text.find('>')?;
    Some((close + 1, rest[..name_length].to_owned()))
}

fn is_url_token_start(text: &str) -> bool {
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let token = text[..end].trim_start_matches(['(', '[', '<', '{', '\'', '"']);
    let token = token.trim_end_matches(is_url_trailing_punctuation);
    let token_lower = token.to_ascii_lowercase();
    if token_lower.starts_with("http://")
        || token_lower.starts_with("https://")
        || token_lower.starts_with("ftp://")
        || token_lower.starts_with("www.")
        || token_lower.contains("://")
    {
        return true;
    }
    if token.is_empty() || token.contains('@') || token.starts_with('.') {
        return false;
    }
    let host = token.split(['/', '?', '#', ':']).next().unwrap_or_default();
    let Some((_, suffix)) = host.rsplit_once('.') else {
        return false;
    };
    suffix.len() >= 2 && suffix.chars().all(|ch| ch.is_ascii_alphabetic())
}

fn is_url_trailing_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '"' | '\''
    )
}

fn is_forbidden_speech_scalar(ch: char) -> bool {
    (ch.is_control() && !ch.is_whitespace())
        || matches!(
            ch as u32,
            0x00AD | 0x034F | 0x061C | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180E
                | 0x200B..=0x200F
                | 0x202A..=0x202E
                | 0x2060..=0x206F
                | 0xFEFF
        )
}

fn strip_markdown_emphasis(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        let marker = if rest.starts_with("**") || rest.starts_with("__") {
            &rest[..2]
        } else if rest.starts_with('*') || rest.starts_with('_') {
            &rest[..1]
        } else {
            let ch = rest.chars().next().expect("valid UTF-8 boundary");
            output.push(ch);
            index += ch.len_utf8();
            continue;
        };
        if rest[marker.len()..].contains(marker) {
            index += marker.len();
        } else {
            output.push_str(marker);
            index += marker.len();
        }
    }
    output
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn split_speech_segments(text: &str) -> Option<Vec<String>> {
    if text.is_empty() {
        return Some(Vec::new());
    }
    let mut sentences = Vec::new();
    let mut start = 0;
    let mut previous = None;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        let next = chars.peek().map(|(_, next)| *next);
        let sentence_end = matches!(ch, '.' | '!' | '?' | '。' | '！' | '？')
            && next.is_none_or(char::is_whitespace)
            && !(ch == '.'
                && previous.is_some_and(|before: char| before.is_ascii_digit())
                && next.is_some_and(|after| after.is_ascii_digit()));
        if sentence_end {
            let end = index + ch.len_utf8();
            if !text[start..end].trim().is_empty() {
                sentences.push(text[start..end].trim());
            }
            start = end;
        }
        previous = Some(ch);
    }
    if start < text.len() && !text[start..].trim().is_empty() {
        sentences.push(text[start..].trim());
    }
    let mut segments = Vec::new();
    for sentence in sentences {
        if !append_bounded_sentence(sentence, &mut segments) {
            return None;
        }
    }
    Some(segments)
}

fn append_bounded_sentence(sentence: &str, segments: &mut Vec<String>) -> bool {
    let mut rest = sentence.trim();
    while !rest.is_empty() {
        if rest.len() <= MAX_SEGMENT_BYTES && rest.chars().count() <= MAX_SEGMENT_SCALARS {
            segments.push(rest.to_owned());
            return true;
        }
        let mut scalar_count = 0;
        let mut byte_limit = 0;
        let mut split = None;
        for (byte, ch) in rest.char_indices() {
            if ch.is_whitespace() {
                if scalar_count > 0 {
                    split = Some(byte);
                }
                continue;
            }
            let next_byte_limit = byte + ch.len_utf8();
            if scalar_count + 1 > MAX_SEGMENT_SCALARS || next_byte_limit > MAX_SEGMENT_BYTES {
                break;
            }
            scalar_count += 1;
            byte_limit = next_byte_limit;
        }
        let split_at = split.filter(|index| *index <= byte_limit).unwrap_or(0);
        if split_at == 0 {
            return false;
        }
        let part = rest[..split_at].trim_end();
        if part.is_empty() {
            return false;
        }
        segments.push(part.to_owned());
        rest = rest[split_at..].trim_start();
    }
    true
}
