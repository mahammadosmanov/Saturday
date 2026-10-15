use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::voice::contract::{
    self, AcceptedUtterance, AckIntentRequest, AppIntentDescriptor, AssetCatalog, AssetError,
    AssetManager, AssetPurpose, AssetRemovalOutcome, AssetState, AssetStatusCode, CancelRequest,
    CaptureError, CaptureLifecycleState, CleanupCode, CleanupReceipt, ConfirmTranscriptRequest,
    DecimalU64, EventQueueError, InputDeviceRecord, Language, LanguageMode, LocalAsrProvider,
    LocalConversationProvider, LocalSpeechProvider, MessageRole, NoticeCode, OperationId,
    OutputProgress, PendingStream, PendingText, ProviderReadiness, ReadinessCode,
    ReadinessSnapshot, ReadinessState, RouteKind, RouterDecision, SelectPreferencesRequest,
    SnapshotBegin, SnapshotEnd, SnapshotFrame, SnapshotPage, SnapshotPageItem, SnapshotSection,
    SnapshotTransferHeader, StartRequest, StatusRequest, SubmitTextRequest, TestOutputRequest,
    TranscriptEligibility, TurnContext, UtteranceSource, VoiceAssetAuthority, VoiceCommand,
    VoiceCommandWireResult, VoiceError, VoiceEventBatch, VoiceEventKind, VoiceEventSender,
    VoiceEventWire, VoiceLanguageIds, VoiceOwner, VoicePhase, VoicePreferences, VoiceResultCode,
    VoiceSnapshot, VoiceSnapshotMetadata, ASR_DEADLINE, GENERATION_DEADLINE, MAX_CAPTURE_SECONDS,
    MAX_EVENT_BATCH_RECORDS, MAX_EVENT_QUEUE_RECORDS, MAX_HISTORY_BYTES, MAX_HISTORY_MESSAGES,
    MAX_MODEL_OUTPUT_BYTES, MAX_SNAPSHOT_METADATA_BYTES, MAX_SNAPSHOT_PAGES,
    MAX_SNAPSHOT_PAGE_METADATA_BYTES, MAX_SNAPSHOT_PAGE_TEXT_BYTES, MAX_SNAPSHOT_TEXT_BYTES,
    MAX_SPOKEN_BYTES, MAX_SPOKEN_SCALARS, MAX_TEXT_DELTA_BYTES, PUBLIC_PROTOCOL_VERSION,
    SPEECH_DEADLINE, WORKER_REAP_DEADLINE,
};
use crate::voice::contract::{NativeCaptureProvider, PendingVoiceEvent};

#[derive(Debug, Clone)]
struct QueuedVoiceEvent {
    operation_id: Option<OperationId>,
    wire: VoiceEventWire,
}

#[derive(Debug, Default)]
struct EventQueueState {
    events: VecDeque<QueuedVoiceEvent>,
    last_sequence: u64,
    revision: u64,
}

/// Fixed-size pull queue shared by native event producers. Producers never wait for the
/// frontend; a dropped event forces a snapshot resynchronization instead of hiding a gap.
pub struct BoundedVoiceEventQueue {
    state: Mutex<EventQueueState>,
    changed: Condvar,
    overflow_generation: AtomicU64,
    acknowledged_overflow_generation: AtomicU64,
}

impl Default for BoundedVoiceEventQueue {
    fn default() -> Self {
        Self {
            state: Mutex::new(EventQueueState::default()),
            changed: Condvar::new(),
            overflow_generation: AtomicU64::new(0),
            acknowledged_overflow_generation: AtomicU64::new(0),
        }
    }
}

impl BoundedVoiceEventQueue {
    pub fn latest_sequence(&self) -> u64 {
        self.state.lock().last_sequence
    }

    pub fn revision(&self) -> u64 {
        self.state.lock().revision
    }

    fn wait_after(&self, after: u64, timeout: Duration) -> Vec<QueuedVoiceEvent> {
        let mut state = self.state.lock();
        if state.last_sequence <= after {
            self.changed.wait_for(&mut state, timeout);
        }
        state
            .events
            .iter()
            .filter(|event| event.wire.event_sequence.0 > after)
            .cloned()
            .collect()
    }

    pub fn batch(&self, after: u64) -> VoiceEventBatch {
        let state = self.state.lock();
        let too_old = state
            .events
            .front()
            .is_some_and(|first| after < first.wire.event_sequence.0.saturating_sub(1));
        let resync_required = after > state.last_sequence
            || too_old
            || self.overflow_generation.load(Ordering::Acquire)
                != self
                    .acknowledged_overflow_generation
                    .load(Ordering::Acquire);
        let events = if resync_required {
            Vec::new()
        } else {
            state
                .events
                .iter()
                .filter(|event| event.wire.event_sequence.0 > after)
                .take(MAX_EVENT_BATCH_RECORDS)
                .map(|event| event.wire.clone())
                .collect()
        };
        VoiceEventBatch {
            protocol_version: PUBLIC_PROTOCOL_VERSION,
            after_event_sequence: DecimalU64(after),
            latest_event_sequence: DecimalU64(state.last_sequence),
            resync_required,
            events,
        }
    }

    /// Call only after a complete snapshot has been atomically committed by the caller.
    /// Concurrent producer overflow remains visible as a newer generation.
    pub fn acknowledge_snapshot(&self, watermark: u64, overflow_generation: u64) -> bool {
        let state = self.state.lock();
        let next_retained = state
            .events
            .front()
            .map_or(state.last_sequence.saturating_add(1), |event| {
                event.wire.event_sequence.0
            });
        if watermark > state.last_sequence
            || next_retained > watermark.saturating_add(1)
            || self.overflow_generation.load(Ordering::Acquire) != overflow_generation
        {
            return false;
        }
        self.acknowledged_overflow_generation
            .store(overflow_generation, Ordering::Release);
        true
    }

    pub fn overflow_generation(&self) -> u64 {
        self.overflow_generation.load(Ordering::Acquire)
    }

    fn record_dropped_event(&self) {
        let _ = self.overflow_generation.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |generation| generation.checked_add(1),
        );
    }
}

impl VoiceEventSender for BoundedVoiceEventQueue {
    fn try_send(&self, pending: PendingVoiceEvent) -> Result<(), EventQueueError> {
        let Some(mut state) = self.state.try_lock() else {
            self.record_dropped_event();
            return Err(EventQueueError::Full);
        };

        if let Some(last) = state.events.back_mut().filter(|last| {
            matches!(&pending.event, VoiceEventKind::CaptureMeter { .. })
                && last.operation_id == pending.operation_id
                && last.wire.owner == pending.owner
                && matches!(&last.wire.event, VoiceEventKind::CaptureMeter { .. })
        }) {
            last.wire.event = pending.event;
            self.changed.notify_one();
            return Ok(());
        }

        let terminal = matches!(
            &pending.event,
            VoiceEventKind::Terminal { .. }
                | VoiceEventKind::Failure { .. }
                | VoiceEventKind::OutputDrained
        );
        let normal_limit = MAX_EVENT_QUEUE_RECORDS.saturating_sub(2);
        if state.events.len() >= normal_limit && !terminal {
            self.record_dropped_event();
            return Err(EventQueueError::Full);
        }
        if state.events.len() == MAX_EVENT_QUEUE_RECORDS {
            let removable = state.events.iter().position(|event| {
                !matches!(
                    &event.wire.event,
                    VoiceEventKind::Terminal { .. }
                        | VoiceEventKind::Failure { .. }
                        | VoiceEventKind::OutputDrained
                )
            });
            let Some(index) = removable else {
                self.record_dropped_event();
                return Err(EventQueueError::Full);
            };
            state.events.remove(index);
            self.record_dropped_event();
        }
        let Some(sequence) = state.last_sequence.checked_add(1) else {
            return Err(EventQueueError::SequenceExhausted);
        };
        let Some(revision) = state.revision.checked_add(1) else {
            return Err(EventQueueError::SequenceExhausted);
        };
        state.last_sequence = sequence;
        state.revision = revision;
        state.events.push_back(QueuedVoiceEvent {
            operation_id: pending.operation_id,
            wire: VoiceEventWire {
                protocol_version: PUBLIC_PROTOCOL_VERSION,
                event_sequence: DecimalU64(sequence),
                revision: DecimalU64(revision),
                owner: pending.owner,
                event: pending.event,
            },
        });
        self.changed.notify_one();
        Ok(())
    }
}

#[cfg(test)]
mod event_queue_tests {
    use super::*;
    use crate::voice::contract::{OperationId, VoiceEventKind};

    fn event(event: VoiceEventKind) -> PendingVoiceEvent {
        PendingVoiceEvent {
            owner: None,
            operation_id: Some(OperationId(1)),
            event,
        }
    }

    #[test]
    fn pull_batches_are_capped_and_overwritten_history_requires_resync() {
        let queue = BoundedVoiceEventQueue::default();
        for _ in 0..MAX_EVENT_QUEUE_RECORDS - 2 {
            queue
                .try_send(event(VoiceEventKind::OutputStarted))
                .unwrap();
        }
        assert_eq!(
            queue.try_send(event(VoiceEventKind::OutputStarted)),
            Err(EventQueueError::Full)
        );
        assert!(queue.overflow_generation() > 0);
        assert!(queue.batch(0).resync_required);

        // A valid snapshot covers the dropped event, allowing retained history to be pulled.
        let watermark = queue.latest_sequence();
        let generation = queue.overflow_generation();
        assert!(queue.acknowledge_snapshot(watermark, generation));
        let capped = queue.batch(0);
        assert!(!capped.resync_required);
        assert_eq!(capped.events.len(), MAX_EVENT_BATCH_RECORDS);
        assert_eq!(capped.events[0].event_sequence.0, 1);
        assert_eq!(
            capped.events[MAX_EVENT_BATCH_RECORDS - 1].event_sequence.0,
            MAX_EVENT_BATCH_RECORDS as u64
        );

        // Further nonterminal sends are dropped and require a fresh snapshot acknowledgment.
        for _ in 0..MAX_EVENT_QUEUE_RECORDS {
            let _ = queue.try_send(event(VoiceEventKind::OutputStarted));
        }
        let old_cursor = queue.batch(0);
        assert!(old_cursor.resync_required);
        assert!(old_cursor.events.is_empty());

        let latest = queue.latest_sequence();
        let generation = queue.overflow_generation();
        assert!(queue.acknowledge_snapshot(latest, generation));
        let recent = queue.batch(latest - 4);
        assert!(!recent.resync_required);
        assert_eq!(recent.events.len(), 4);
        assert_eq!(recent.events[0].event_sequence.0, latest - 3);
        assert_eq!(recent.events[3].event_sequence.0, latest);
    }

    #[test]
    fn terminal_records_remain_pullable_and_overflow_requires_snapshot_ack() {
        let queue = BoundedVoiceEventQueue::default();
        for _ in 0..MAX_EVENT_QUEUE_RECORDS - 2 {
            queue
                .try_send(event(VoiceEventKind::OutputStarted))
                .unwrap();
        }
        assert_eq!(
            queue.try_send(event(VoiceEventKind::OutputStarted)),
            Err(EventQueueError::Full)
        );
        let stale_generation = queue.overflow_generation();
        queue
            .try_send(event(VoiceEventKind::OutputDrained))
            .unwrap();
        queue
            .try_send(event(VoiceEventKind::OutputDrained))
            .unwrap();
        let terminal_sequence = queue.latest_sequence();
        queue
            .try_send(event(VoiceEventKind::OutputDrained))
            .unwrap();
        assert_eq!(queue.latest_sequence(), terminal_sequence + 1);

        let generation = queue.overflow_generation();
        assert!(generation > stale_generation);
        assert!(queue.batch(0).resync_required);
        assert!(!queue.acknowledge_snapshot(terminal_sequence + 1, stale_generation));
        assert!(!queue.acknowledge_snapshot(terminal_sequence + 2, generation));
        assert!(!queue.acknowledge_snapshot(0, generation));
        assert!(queue.acknowledge_snapshot(terminal_sequence, generation));
        let batch = queue.batch(terminal_sequence);
        assert!(!batch.resync_required);
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].event_sequence.0, terminal_sequence + 1);
        assert!(matches!(
            batch.events[0].event,
            VoiceEventKind::OutputDrained
        ));
    }

    #[test]
    fn producers_do_not_block_and_dropped_events_force_resynchronization() {
        let queue = BoundedVoiceEventQueue::default();
        let _state = queue.state.lock();
        assert_eq!(
            queue.try_send(event(VoiceEventKind::OutputDrained)),
            Err(EventQueueError::Full)
        );
        assert!(queue.overflow_generation() > 0);
    }
    #[test]
    fn internal_capture_events_keep_their_operation_identity() {
        let queue = BoundedVoiceEventQueue::default();
        let owner = VoiceOwner {
            session_id: "session".to_owned(),
            turn_id: "turn".to_owned(),
            generation: 1,
        };
        queue
            .try_send(PendingVoiceEvent {
                owner: Some(owner),
                operation_id: Some(OperationId(23)),
                event: VoiceEventKind::CaptureLifecycle {
                    state: CaptureLifecycleState::DeviceLost,
                },
            })
            .unwrap();

        let queued = queue.wait_after(0, Duration::ZERO);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].operation_id, Some(OperationId(23)));
    }
}

#[cfg(test)]
mod device_loss_tests {
    use super::*;

    fn owner(generation: u64) -> VoiceOwner {
        VoiceOwner {
            session_id: "device-loss-session".to_owned(),
            turn_id: format!("turn-{generation}"),
            generation,
        }
    }

    fn active_state(
        owner: VoiceOwner,
        operation_id: OperationId,
        events: Arc<BoundedVoiceEventQueue>,
    ) -> Mutex<SessionState> {
        let event_sender: Arc<dyn VoiceEventSender> = events;
        let context = TurnContext {
            owner: owner.clone(),
            operation_id,
            cancel: contract::CancelToken::default(),
            deadline: Instant::now() + Duration::from_secs(30),
            event_sender,
        };
        let mut snapshot = initial_snapshot(
            "device-loss-session".to_owned(),
            initial_readiness(None, false, false, false),
            Vec::new(),
            Vec::new(),
            0,
        );
        snapshot.owner = Some(owner);
        snapshot.metadata.phase = VoicePhase::Listening;
        Mutex::new(SessionState {
            snapshot,
            history: crate::voice::context::CompletedTurnStore::default(),
            active: Some(ActiveTurn {
                context,
                capture_operation_id: Some(operation_id),
                language_mode: LanguageMode::En,
                source: UtteranceSource::Speech,
            }),
            pending_review: None,
            pending_intent: None,
            generation: operation_id.0,
            next_operation_id: operation_id.0 + 1,
            next_transfer_id: 1,
            applied_event_sequence: 0,
            applied_revision: 0,
            pinned_snapshot: None,
            locked: false,
            shutting_down: false,
        })
    }

    fn fake_start(
        provider_session: &mut Option<(VoiceOwner, OperationId)>,
        owner: VoiceOwner,
        operation_id: OperationId,
    ) -> bool {
        if provider_session.is_some() {
            return false;
        }
        *provider_session = Some((owner, operation_id));
        true
    }

    #[test]
    fn device_loss_releases_provider_session_before_subsequent_capture_can_start() {
        let session_owner = owner(1);
        let operation_id = OperationId(7);
        let events = Arc::new(BoundedVoiceEventQueue::default());
        let state = active_state(session_owner.clone(), operation_id, Arc::clone(&events));
        let changed = Condvar::new();
        let mut provider_session = Some((session_owner.clone(), operation_id));
        assert!(!fake_start(&mut provider_session, owner(2), OperationId(8)));

        apply_device_loss_event(
            &state,
            &events,
            &changed,
            &session_owner,
            operation_id,
            |event_owner: &VoiceOwner, event_operation_id: OperationId| {
                let turn_matches = {
                    let state = state.lock();
                    let active = state
                        .active
                        .as_ref()
                        .expect("turn remains active until release");
                    active.context.owner == *event_owner
                        && active.context.operation_id == event_operation_id
                        && active.context.cancel.is_cancelled()
                };
                assert!(turn_matches);
                assert!(
                    provider_session
                        .as_ref()
                        .is_some_and(|(active_owner, active_id)| {
                            active_owner == event_owner && *active_id == event_operation_id
                        }),
                    "release must target this event's exact provider session"
                );
                provider_session = None;
                CleanupReceipt::verified(CleanupCode::CaptureStopped)
            },
        );

        assert!(
            provider_session.is_none(),
            "device loss released the stale provider session"
        );
        assert!(
            state.lock().active.is_none(),
            "the supervisor turn clears after release"
        );
        assert!(
            fake_start(&mut provider_session, owner(2), OperationId(8)),
            "a new capture can start after confirmed release"
        );
        assert_eq!(
            state.lock().snapshot.metadata.cleanup,
            CleanupReceipt::verified(CleanupCode::CaptureStopped)
        );
    }

    #[test]
    fn queued_device_loss_matches_capture_id_after_finish_allocates_transcription_id() {
        let owner = owner(1);
        let capture_operation_id = OperationId(7);
        let transcription_operation_id = OperationId(8);
        let events = Arc::new(BoundedVoiceEventQueue::default());
        let state = active_state(owner.clone(), capture_operation_id, Arc::clone(&events));
        let changed = Condvar::new();
        let mut provider_session = Some((owner.clone(), capture_operation_id));

        {
            let mut state = state.lock();
            let active = state.active.as_mut().expect("capture turn remains active");
            active.context.operation_id = transcription_operation_id;
            state.snapshot.metadata.phase = VoicePhase::Error;
            state.snapshot.metadata.cleanup = CleanupReceipt::failed();
            state.snapshot.metadata.error = Some(VoiceError::CleanupFailed);
            state.locked = true;
        }

        apply_device_loss_event(
            &state,
            &events,
            &changed,
            &owner,
            capture_operation_id,
            |event_owner, event_operation_id| {
                let state_guard = state.lock();
                let active = state_guard
                    .active
                    .as_ref()
                    .expect("turn remains active until release");
                assert_eq!(active.context.operation_id, transcription_operation_id);
                assert!(active.context.cancel.is_cancelled());
                assert_eq!(event_operation_id, capture_operation_id);
                assert_eq!(event_owner, &owner);
                assert_eq!(
                    provider_session,
                    Some((owner.clone(), capture_operation_id)),
                    "the queued event releases its original capture operation"
                );
                drop(state_guard);
                provider_session = None;
                CleanupReceipt::verified(CleanupCode::CaptureStopped)
            },
        );

        let state = state.lock();
        assert!(state.active.is_none());
        assert!(!state.locked, "verified capture release permits recovery");
        assert_eq!(
            state.snapshot.metadata.cleanup,
            CleanupReceipt::verified(CleanupCode::CaptureStopped)
        );
    }

    #[test]
    fn device_loss_does_not_acknowledge_or_clear_turn_when_provider_release_fails() {
        let owner = owner(1);
        let operation_id = OperationId(7);
        let events = Arc::new(BoundedVoiceEventQueue::default());
        let state = active_state(owner.clone(), operation_id, Arc::clone(&events));
        let changed = Condvar::new();

        apply_device_loss_event(&state, &events, &changed, &owner, operation_id, |_, _| {
            CleanupReceipt::failed()
        });

        let state = state.lock();
        assert!(
            state.active.is_some(),
            "an unconfirmed native session must retain its turn"
        );
        assert!(state.locked, "failed native release locks further capture");
        assert!(!state.snapshot.metadata.cleanup.verified);
        assert_eq!(
            state.snapshot.metadata.error,
            Some(VoiceError::CleanupFailed)
        );
    }
    #[test]
    fn stale_device_loss_operation_cannot_cancel_the_current_turn() {
        let owner = owner(1);
        let current_operation_id = OperationId(8);
        let stale_operation_id = OperationId(7);
        let events = Arc::new(BoundedVoiceEventQueue::default());
        let state = active_state(owner.clone(), current_operation_id, Arc::clone(&events));
        let changed = Condvar::new();
        let mut release_called = false;

        apply_device_loss_event(
            &state,
            &events,
            &changed,
            &owner,
            stale_operation_id,
            |_, _| {
                release_called = true;
                CleanupReceipt::verified(CleanupCode::CaptureStopped)
            },
        );

        assert!(!release_called);
        let state = state.lock();
        assert_eq!(
            state
                .active
                .as_ref()
                .map(|active| active.context.operation_id),
            Some(current_operation_id)
        );
        assert!(!state.locked);
        assert_eq!(state.snapshot.metadata.phase, VoicePhase::Listening);
    }
    #[test]
    fn concurrent_reconnect_is_not_cleared_by_an_inflight_device_loss_event() {
        let old_owner = owner(1);
        let old_operation_id = OperationId(7);
        let new_owner = owner(2);
        let new_operation_id = OperationId(8);
        let events = Arc::new(BoundedVoiceEventQueue::default());
        let state = active_state(old_owner.clone(), old_operation_id, Arc::clone(&events));
        let changed = Condvar::new();
        let mut provider_session = Some((old_owner.clone(), old_operation_id));

        apply_device_loss_event(
            &state,
            &events,
            &changed,
            &old_owner,
            old_operation_id,
            |event_owner, event_operation_id| {
                assert_eq!(event_owner, &old_owner);
                assert_eq!(event_operation_id, old_operation_id);
                {
                    let mut state = state.lock();
                    state.snapshot.owner = Some(new_owner.clone());
                    let active = state.active.as_mut().expect("replacement turn is active");
                    active.context.owner = new_owner.clone();
                    active.context.operation_id = new_operation_id;
                    active.context.cancel = contract::CancelToken::default();
                }
                provider_session = Some((new_owner.clone(), new_operation_id));
                CleanupReceipt::failed()
            },
        );

        assert_eq!(
            provider_session,
            Some((new_owner.clone(), new_operation_id)),
            "the old event must not release the replacement provider session"
        );
        let state = state.lock();
        let active = state
            .active
            .as_ref()
            .expect("replacement turn remains active");
        assert_eq!(active.context.owner, new_owner);
        assert_eq!(active.context.operation_id, new_operation_id);
        assert!(!state.locked);
        assert_eq!(state.snapshot.metadata.error, None);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeStopReason {
    WindowHidden,
    WindowUnfocused,
    WindowClosed,
    ApplicationLocked,
    ApplicationExit,
    AssetRevoked,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum VoiceStatusRead {
    Snapshot(Box<SnapshotFrame>),
    Events(VoiceEventBatch),
}

#[derive(Clone)]
struct ActiveTurn {
    context: TurnContext,
    capture_operation_id: Option<OperationId>,
    language_mode: LanguageMode,
    source: UtteranceSource,
}

struct PendingReview {
    owner: VoiceOwner,
    text: String,
}

struct PendingIntent {
    descriptor: AppIntentDescriptor,
    utterance: AcceptedUtterance,
    response_language: Language,
}

struct PinnedSnapshot {
    caller_origin: String,
    expires_at: Instant,
    watermark: u64,
    overflow_generation: u64,
    next_cursor: usize,
    begin: SnapshotBegin,
    pages: Vec<SnapshotPage>,
    end: SnapshotEnd,
}

struct SessionState {
    snapshot: VoiceSnapshot,
    history: crate::voice::context::CompletedTurnStore,
    active: Option<ActiveTurn>,
    pending_review: Option<PendingReview>,
    pending_intent: Option<PendingIntent>,
    generation: u64,
    next_operation_id: u64,
    next_transfer_id: u64,
    applied_event_sequence: u64,
    applied_revision: u64,
    pinned_snapshot: Option<PinnedSnapshot>,
    locked: bool,
    shutting_down: bool,
}

struct VoiceShared {
    state: Mutex<SessionState>,
    changed: Condvar,
    events: Arc<BoundedVoiceEventQueue>,
    monitor_shutdown: std::sync::atomic::AtomicBool,
    worker_session: Mutex<Option<Arc<crate::voice_authorization::AuthorizedWorkerSession>>>,
    worker_authority: Arc<crate::voice_authorization::VoiceAuthorizationService>,
}

pub struct CleanupWaitHandle {
    shared: Arc<VoiceShared>,
}

pub struct VoiceSessionSupervisor {
    shared: Arc<VoiceShared>,
    asset_manager: Option<Arc<dyn AssetManager>>,
    authorization: Arc<crate::voice_authorization::VoiceAuthorizationService>,
    capture: Arc<Mutex<Option<crate::voice::capture::NativeAudioCaptureProvider>>>,
    asr: Arc<Mutex<Option<crate::voice::asr::NativeAsrProvider>>>,
    speech: Arc<Mutex<Option<crate::voice::speech::SystemSpeechProvider>>>,
    authority: CallerVoiceAuthority,
    event_monitor: Mutex<Option<thread::JoinHandle<()>>>,
}

struct CallerVoiceAuthority {
    next_generation: AtomicU64,
}

impl Default for CallerVoiceAuthority {
    fn default() -> Self {
        Self {
            next_generation: AtomicU64::new(1),
        }
    }
}

impl contract::VoiceAuthority for CallerVoiceAuthority {
    fn authorize(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
        _command: &VoiceCommand,
    ) -> Result<contract::NativeVoiceGrant, VoiceError> {
        if !caller.is_trusted() {
            return Err(VoiceError::Unauthorized);
        }
        let generation = self
            .next_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .map_err(|_| VoiceError::SequenceExhausted)?;
        Ok(contract::NativeVoiceGrant { generation })
    }
}

type AssetManagerOptions = (
    Option<Arc<dyn AssetManager>>,
    bool,
    Arc<crate::voice_authorization::VoiceAuthorizationService>,
);

impl VoiceSessionSupervisor {
    pub fn from_app(app: tauri::AppHandle) -> Result<Self, VoiceError> {
        let (asset_manager, trust_setup_required, authorization) =
            Self::build_asset_manager(app.clone())?;
        let capture = crate::voice::capture::NativeAudioCaptureProvider::new().ok();
        let speech = None;
        let input_devices = capture
            .as_ref()
            .and_then(|provider| contract::NativeCaptureProvider::enumerate_devices(provider).ok())
            .map(|catalog| catalog.devices)
            .unwrap_or_default();
        let (voices, voice_revision) =
            crate::platform::voice_speech::NativeSpeech::enumerate_voices()
                .ok()
                .and_then(|(records, revision)| {
                    crate::voice::speech::voice_catalog_from_native(records, revision)
                        .ok()
                        .map(|catalog| {
                            let mut voices = catalog.turkish;
                            voices.extend(catalog.english);
                            (voices, catalog.revision)
                        })
                })
                .unwrap_or_default();
        let readiness = initial_readiness(
            asset_manager.as_deref(),
            capture.is_some() && !input_devices.is_empty(),
            false,
            trust_setup_required,
        );
        let capture = Arc::new(Mutex::new(capture));
        let session_id = match fresh_session_id() {
            Ok(id) => id,
            Err(_) => "session-unavailable".to_owned(),
        };
        let snapshot =
            initial_snapshot(session_id, readiness, input_devices, voices, voice_revision);
        let events = Arc::new(BoundedVoiceEventQueue::default());
        let shared = Arc::new(VoiceShared {
            state: Mutex::new(SessionState {
                snapshot,
                history: crate::voice::context::CompletedTurnStore::default(),
                active: None,
                pending_review: None,
                pending_intent: None,
                generation: 0,
                next_operation_id: 1,
                next_transfer_id: 1,
                applied_event_sequence: 0,
                applied_revision: 0,
                pinned_snapshot: None,
                locked: false,
                shutting_down: false,
            }),
            changed: Condvar::new(),
            events,
            monitor_shutdown: std::sync::atomic::AtomicBool::new(false),
            worker_session: Mutex::new(None),
            worker_authority: Arc::clone(&authorization),
        });
        let event_monitor = Some(spawn_event_monitor(
            Arc::downgrade(&shared),
            Arc::clone(&capture),
        ));
        Ok(Self {
            shared,
            authorization,
            asr: Arc::new(Mutex::new(asset_manager.as_ref().map(|assets| {
                crate::voice::asr::NativeAsrProvider::new(Arc::clone(assets))
            }))),
            asset_manager,
            capture,
            speech: Arc::new(Mutex::new(speech)),
            authority: CallerVoiceAuthority::default(),
            event_monitor: Mutex::new(event_monitor),
        })
    }

    fn build_asset_manager(app: tauri::AppHandle) -> Result<AssetManagerOptions, VoiceError> {
        let authorization = Arc::new(
            crate::voice_authorization::VoiceAuthorizationService::from_app(app.clone())
                .map_err(|_| VoiceError::Unavailable)?,
        );
        let prompt: Arc<dyn contract::VoiceConsentPrompt> = Arc::new(
            crate::voice_authorization::NativeVoiceConsentPrompt::new(app.clone()),
        );
        let asset_manager: Arc<dyn AssetManager> = Arc::new(
            crate::voice::assets::RootAssetManager::new(
                app,
                Arc::clone(&authorization),
                prompt,
                authorization.clone(),
            )
            .map_err(map_asset_error)?,
        );
        let trust_setup_required = asset_manager
            .inventory()
            .map_err(map_asset_error)?
            .assets
            .iter()
            .any(|asset| asset.code == AssetStatusCode::OwnerTrustSetupRequired);
        Ok((Some(asset_manager), trust_setup_required, authorization))
    }

    pub fn bootstrap_voice_owner_key(
        &self,
    ) -> Result<(&'static str, String), crate::voice_authorization::VoiceAuthorizationError> {
        match self.authorization.bootstrap_owner_for_current_profile()? {
            crate::voice_authorization::VoiceOwnerBootstrapResult::BootstrapAnchorCreated(
                review,
            ) => Ok(("bootstrap_anchor", review.owner_public_key_sha256)),
            crate::voice_authorization::VoiceOwnerBootstrapResult::FinalBinding(review) => {
                Ok(("final_binding", review.binding_sha256))
            }
        }
    }

    pub fn cleanup_wait_handle(&self) -> CleanupWaitHandle {
        CleanupWaitHandle {
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn cleanup_receipt(&self) -> CleanupReceipt {
        self.cleanup_wait_handle().wait_for_cleanup(Duration::ZERO)
    }
}

fn fresh_session_id() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)?;
    let mut text = String::with_capacity(32);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(text)
}

fn initial_snapshot(
    session_id: String,
    readiness: ReadinessSnapshot,
    input_devices: Vec<InputDeviceRecord>,
    voices: Vec<contract::VoiceRecord>,
    voice_revision: u64,
) -> VoiceSnapshot {
    let phase = if readiness.product_ready {
        VoicePhase::Idle
    } else {
        VoicePhase::SetupRequired
    };
    VoiceSnapshot {
        metadata: VoiceSnapshotMetadata {
            session_id,
            phase,
            preferences: VoicePreferences::default(),
            resolved_language: None,
            readiness,
            voice_catalog: contract::VoiceCatalogRevision {
                revision: voice_revision,
            },
            cancel_available: false,
            cleanup: CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
            route_kind: None,
            capture_elapsed_ms: None,
            output_progress: OutputProgress {
                queued_segments: 0,
                completed_segments: 0,
                active: false,
            },
            next_transcript_sequence: 1,
            next_answer_sequence: 1,
            notices: Vec::new(),
            error: None,
        },
        owner: None,
        history: Vec::new(),
        pending: Vec::new(),
        input_devices,
        voices,
    }
}

fn readiness_from_asset(manager: Option<&dyn AssetManager>, asset_id: &str) -> ProviderReadiness {
    let Some(manager) = manager else {
        return ProviderReadiness {
            state: ReadinessState::Unavailable,
            code: ReadinessCode::ProviderUnavailable,
        };
    };
    let Ok(catalog) = manager.inventory() else {
        return ProviderReadiness {
            state: ReadinessState::Unavailable,
            code: ReadinessCode::ProviderUnavailable,
        };
    };
    let Some(asset) = catalog
        .assets
        .iter()
        .find(|asset| asset.asset_id == asset_id)
    else {
        return ProviderReadiness {
            state: ReadinessState::Unavailable,
            code: ReadinessCode::AssetUnavailable,
        };
    };
    if asset.status == AssetState::Ready && asset.code == AssetStatusCode::Verified {
        return ProviderReadiness {
            state: ReadinessState::Ready,
            code: ReadinessCode::Ready,
        };
    }
    let code = match asset.code {
        AssetStatusCode::NotReviewed | AssetStatusCode::SourceReviewRequired => {
            ReadinessCode::SourceReviewRequired
        }
        AssetStatusCode::RuntimeBuildRequired => ReadinessCode::RuntimeBuildRequired,
        AssetStatusCode::OwnerTrustSetupRequired => ReadinessCode::OwnerTrustSetupRequired,
        AssetStatusCode::ModelApprovalRequired | AssetStatusCode::TransferNotGranted => {
            ReadinessCode::AssetApprovalRequired
        }
        AssetStatusCode::AssetMissing
        | AssetStatusCode::Disabled
        | AssetStatusCode::RecoveryRequired => ReadinessCode::AssetUnavailable,
        AssetStatusCode::Verified => ReadinessCode::ProviderUnavailable,
    };
    ProviderReadiness {
        state: ReadinessState::SetupRequired,
        code,
    }
}

fn initial_readiness(
    manager: Option<&dyn AssetManager>,
    capture_available: bool,
    speech_available: bool,
    trust_setup_required: bool,
) -> ReadinessSnapshot {
    let mut asr = readiness_from_asset(manager, "whisper-base");
    let mut llm = readiness_from_asset(manager, "qwen3-4b-q4-k-m");
    let capture = if capture_available {
        ProviderReadiness {
            state: ReadinessState::SetupRequired,
            code: ReadinessCode::PermissionRequired,
        }
    } else {
        ProviderReadiness {
            state: ReadinessState::Unavailable,
            code: ReadinessCode::ProviderUnavailable,
        }
    };
    let speech = if speech_available {
        ProviderReadiness {
            state: ReadinessState::SetupRequired,
            code: ReadinessCode::OfflineEvidenceRequired,
        }
    } else {
        ProviderReadiness {
            state: ReadinessState::Unavailable,
            code: ReadinessCode::ProviderUnavailable,
        }
    };
    if manager.is_none() {
        let code = if trust_setup_required {
            ReadinessCode::OwnerTrustSetupRequired
        } else {
            ReadinessCode::SourceReviewRequired
        };
        asr = ProviderReadiness {
            state: ReadinessState::SetupRequired,
            code,
        };
        llm = asr.clone();
    }
    let router = ProviderReadiness {
        state: ReadinessState::Ready,
        code: ReadinessCode::Ready,
    };
    let product_ready = capture.state == ReadinessState::Ready
        && asr.state == ReadinessState::Ready
        && llm.state == ReadinessState::Ready
        && speech.state == ReadinessState::Ready;
    ReadinessSnapshot {
        capture,
        asr,
        router,
        llm,
        speech,
        routine_tasks: contract::RoutineTaskCounters {
            completed: 0,
            without_llm: 0,
        },
        product_ready,
    }
}

fn fresh_snapshot_text_bytes(snapshot: &VoiceSnapshot) -> usize {
    snapshot
        .history
        .iter()
        .map(|message| message.text.len())
        .sum::<usize>()
        + snapshot
            .pending
            .iter()
            .map(|pending| pending.text.len())
            .sum::<usize>()
}

#[derive(Serialize)]
struct SnapshotDigestMaterial {
    version: &'static str,
    metadata: VoiceSnapshotMetadata,
    owner: Option<VoiceOwner>,
    history: Vec<contract::HistoryMessage>,
    pending: Vec<PendingText>,
    input_devices: Vec<InputDeviceRecord>,
    voices: Vec<contract::VoiceRecord>,
}

fn snapshot_digest_material(snapshot: &VoiceSnapshot) -> SnapshotDigestMaterial {
    let history = snapshot
        .history
        .iter()
        .cloned()
        .map(|mut message| {
            message.text.clear();
            message
        })
        .collect();
    let pending = snapshot
        .pending
        .iter()
        .cloned()
        .map(|mut item| {
            item.text.clear();
            item
        })
        .collect();
    SnapshotDigestMaterial {
        version: "saturday.voice.snapshot.digest.v1",
        metadata: snapshot.metadata.clone(),
        owner: snapshot.owner.clone(),
        history,
        pending,
        input_devices: snapshot.input_devices.clone(),
        voices: snapshot.voices.clone(),
    }
}

fn digest_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn split_utf8(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + MAX_SNAPSHOT_PAGE_TEXT_BYTES).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(text[start..end].to_owned());
        start = end;
    }
    chunks
}

fn sanitized_page(page: &SnapshotPage) -> SnapshotPage {
    let items = page
        .items
        .iter()
        .cloned()
        .map(|mut item| {
            match &mut item {
                SnapshotPageItem::HistoryChunk { text, .. }
                | SnapshotPageItem::PendingChunk { text, .. } => text.clear(),
                _ => {}
            }
            item
        })
        .collect();
    SnapshotPage {
        protocol_version: page.protocol_version,
        event_sequence: page.event_sequence,
        resync_id: page.resync_id,
        revision: page.revision,
        owner: page.owner.clone(),
        index: page.index,
        section: page.section,
        items,
    }
}

type SnapshotTransfer = (SnapshotBegin, Vec<SnapshotPage>, SnapshotEnd);

fn build_snapshot_transfer(
    snapshot: &VoiceSnapshot,
    watermark: u64,
    revision: u64,
    resync_id: u64,
) -> Result<SnapshotTransfer, VoiceError> {
    if snapshot.history.len() > MAX_HISTORY_MESSAGES
        || snapshot
            .history
            .iter()
            .map(|message| message.text.len())
            .sum::<usize>()
            > MAX_HISTORY_BYTES
        || fresh_snapshot_text_bytes(snapshot) > MAX_SNAPSHOT_TEXT_BYTES
    {
        return Err(VoiceError::TooLarge);
    }
    let digest_material = snapshot_digest_material(snapshot);
    let metadata_bytes = serde_json::to_vec(&digest_material).map_err(|_| VoiceError::Internal)?;
    if metadata_bytes.len() > MAX_SNAPSHOT_METADATA_BYTES {
        return Err(VoiceError::TooLarge);
    }
    let text_bytes = fresh_snapshot_text_bytes(snapshot);
    let owner = snapshot.owner.clone();
    let revision = DecimalU64(revision);
    let event_sequence = DecimalU64(watermark);
    let mut items = Vec::<(SnapshotSection, SnapshotPageItem, usize)>::new();
    for (index, message) in snapshot.history.iter().enumerate() {
        let chunks = split_utf8(&message.text);
        items.push((
            SnapshotSection::History,
            SnapshotPageItem::HistoryMessage {
                message_index: index as u16,
                id: message.id.clone(),
                role: message.role,
                language: message.language,
                source: message.source,
                total_bytes: message.text.len() as u32,
                chunk_count: chunks.len() as u16,
            },
            0,
        ));
        let mut offset = 0u32;
        let total_bytes = message.text.len() as u32;
        let chunk_count = chunks.len() as u16;
        for (chunk_index, text) in chunks.into_iter().enumerate() {
            let bytes = text.len();
            items.push((
                SnapshotSection::History,
                SnapshotPageItem::HistoryChunk {
                    message_index: index as u16,
                    byte_offset: offset,
                    total_bytes,
                    chunk_index: chunk_index as u16,
                    chunk_count,
                    text,
                },
                bytes,
            ));
            offset = offset
                .checked_add(bytes as u32)
                .ok_or(VoiceError::TooLarge)?;
        }
    }
    for pending in &snapshot.pending {
        let chunks = split_utf8(&pending.text);
        let total_bytes = pending.text.len() as u32;
        let chunk_count = chunks.len() as u16;
        let mut offset = 0u32;
        for (chunk_index, text) in chunks.into_iter().enumerate() {
            let bytes = text.len();
            items.push((
                SnapshotSection::Pending,
                SnapshotPageItem::PendingChunk {
                    stream: pending.stream,
                    sequence: pending.sequence,
                    language: pending.language,
                    source: pending.source,
                    byte_offset: offset,
                    total_bytes,
                    chunk_index: chunk_index as u16,
                    chunk_count,
                    final_chunk: pending.final_chunk,
                    partial: pending.partial,
                    text,
                },
                bytes,
            ));
            offset = offset
                .checked_add(bytes as u32)
                .ok_or(VoiceError::TooLarge)?;
        }
    }
    for device in &snapshot.input_devices {
        items.push((
            SnapshotSection::InputDevices,
            SnapshotPageItem::InputDevice {
                record: device.clone(),
            },
            0,
        ));
    }
    for voice in &snapshot.voices {
        items.push((
            SnapshotSection::Voices,
            SnapshotPageItem::Voice {
                record: voice.clone(),
            },
            0,
        ));
    }
    let mut pages = Vec::<SnapshotPage>::new();
    let mut section = None;
    let mut page_items = Vec::<SnapshotPageItem>::new();
    let mut page_text_bytes = 0usize;
    let make_page =
        |index: usize, section: SnapshotSection, page_items: Vec<SnapshotPageItem>| SnapshotPage {
            protocol_version: PUBLIC_PROTOCOL_VERSION,
            event_sequence,
            resync_id: DecimalU64(resync_id),
            revision,
            owner: owner.clone(),
            index: index as u16,
            section,
            items: page_items,
        };
    for (next_section, item, item_text_bytes) in items {
        let section_changed = section.is_some_and(|current| current != next_section);
        if section_changed {
            let previous = section.take().ok_or(VoiceError::Internal)?;
            pages.push(make_page(
                pages.len(),
                previous,
                std::mem::take(&mut page_items),
            ));
            page_text_bytes = 0;
        }
        section = Some(next_section);
        let mut candidate = page_items.clone();
        candidate.push(item.clone());
        let candidate_page = make_page(pages.len(), next_section, candidate);
        let metadata_len = serde_json::to_vec(&SnapshotFrame::Page(Box::new(sanitized_page(
            &candidate_page,
        ))))
        .map_err(|_| VoiceError::Internal)?
        .len();
        let frame_len = serde_json::to_vec(&SnapshotFrame::Page(Box::new(candidate_page.clone())))
            .map_err(|_| VoiceError::Internal)?
            .len();
        if !page_items.is_empty()
            && (page_text_bytes + item_text_bytes > MAX_SNAPSHOT_PAGE_TEXT_BYTES
                || metadata_len > MAX_SNAPSHOT_PAGE_METADATA_BYTES
                || frame_len > contract::MAX_CONTROL_FRAME_BYTES)
        {
            pages.push(make_page(
                pages.len(),
                next_section,
                std::mem::take(&mut page_items),
            ));
            page_text_bytes = 0;
        }
        let mut single = vec![item.clone()];
        let single_page = make_page(pages.len(), next_section, std::mem::take(&mut single));
        let single_metadata_len =
            serde_json::to_vec(&SnapshotFrame::Page(Box::new(sanitized_page(&single_page))))
                .map_err(|_| VoiceError::Internal)?
                .len();
        let single_frame_len = serde_json::to_vec(&SnapshotFrame::Page(Box::new(single_page)))
            .map_err(|_| VoiceError::Internal)?
            .len();
        if item_text_bytes > MAX_SNAPSHOT_PAGE_TEXT_BYTES
            || single_metadata_len > MAX_SNAPSHOT_PAGE_METADATA_BYTES
            || single_frame_len > contract::MAX_CONTROL_FRAME_BYTES
        {
            return Err(VoiceError::TooLarge);
        }
        page_items.push(item);
        page_text_bytes += item_text_bytes;
    }
    if let Some(section) = section {
        pages.push(make_page(pages.len(), section, page_items));
    }
    if pages.len() > MAX_SNAPSHOT_PAGES {
        return Err(VoiceError::TooLarge);
    }
    let digest_sha256 = digest_hex(&metadata_bytes);
    let begin = SnapshotBegin {
        protocol_version: PUBLIC_PROTOCOL_VERSION,
        event_sequence,
        resync_id: DecimalU64(resync_id),
        revision,
        owner: owner.clone(),
        page_count: pages.len() as u16,
        total_text_bytes: text_bytes as u32,
        total_metadata_bytes: metadata_bytes.len() as u32,
        digest_sha256: digest_sha256.clone(),
        metadata: snapshot.metadata.clone(),
    };
    let end = SnapshotEnd {
        protocol_version: PUBLIC_PROTOCOL_VERSION,
        event_sequence,
        resync_id: DecimalU64(resync_id),
        revision,
        owner,
        page_count: pages.len() as u16,
        total_text_bytes: text_bytes as u32,
        total_metadata_bytes: metadata_bytes.len() as u32,
        digest_sha256,
    };
    let begin_len = serde_json::to_vec(&SnapshotFrame::Begin(Box::new(begin.clone())))
        .map_err(|_| VoiceError::Internal)?
        .len();
    let end_len = serde_json::to_vec(&SnapshotFrame::End(end.clone()))
        .map_err(|_| VoiceError::Internal)?
        .len();
    if begin_len > contract::MAX_CONTROL_FRAME_BYTES || end_len > contract::MAX_CONTROL_FRAME_BYTES
    {
        return Err(VoiceError::TooLarge);
    }
    Ok((begin, pages, end))
}

impl VoiceSessionSupervisor {
    pub fn authorize(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
        command: &VoiceCommand,
    ) -> Result<(), VoiceError> {
        let grant = contract::VoiceAuthority::authorize(&self.authority, caller, command)?;
        if grant.generation == 0 {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }

    pub fn get_status(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        request: StatusRequest,
    ) -> Result<VoiceStatusRead, VoiceError> {
        if !caller.is_trusted() {
            return Err(VoiceError::Unauthorized);
        }
        match request {
            StatusRequest::SnapshotBegin {} => self
                .begin_snapshot(caller)
                .map(|frame| VoiceStatusRead::Snapshot(Box::new(frame))),
            StatusRequest::SnapshotNext { resync_id, cursor } => self
                .next_snapshot(caller, resync_id.0, cursor.0)
                .map(|frame| VoiceStatusRead::Snapshot(Box::new(frame))),
            StatusRequest::Events {
                after_event_sequence,
            } => Ok(VoiceStatusRead::Events(
                self.shared.events.batch(after_event_sequence.0),
            )),
        }
    }

    fn begin_snapshot(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
    ) -> Result<SnapshotFrame, VoiceError> {
        let mut state = self.shared.state.lock();
        if state.pinned_snapshot.as_ref().is_some_and(|transfer| {
            transfer.expires_at > Instant::now() && transfer.caller_origin != caller.origin()
        }) {
            return Err(VoiceError::Busy);
        }
        let resync_id = state.next_transfer_id;
        state.next_transfer_id = state
            .next_transfer_id
            .checked_add(1)
            .ok_or(VoiceError::SequenceExhausted)?;
        let snapshot = state.snapshot.clone();
        let (watermark, revision, overflow_generation) = (
            state.applied_event_sequence,
            state.applied_revision,
            self.shared.events.overflow_generation(),
        );
        let (begin, pages, end) =
            build_snapshot_transfer(&snapshot, watermark, revision, resync_id)?;
        let frame = SnapshotFrame::Begin(Box::new(begin.clone()));
        state.pinned_snapshot = Some(PinnedSnapshot {
            caller_origin: caller.origin().to_owned(),
            expires_at: Instant::now() + Duration::from_secs(30),
            watermark,
            overflow_generation,
            next_cursor: 0,
            begin,
            pages,
            end,
        });
        Ok(frame)
    }

    fn next_snapshot(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        resync_id: u64,
        cursor: u64,
    ) -> Result<SnapshotFrame, VoiceError> {
        let mut state = self.shared.state.lock();
        let Some(transfer) = state.pinned_snapshot.as_mut() else {
            return Err(VoiceError::StaleCatalog);
        };
        if transfer.expires_at <= Instant::now()
            || transfer.caller_origin != caller.origin()
            || transfer.begin.resync_id.0 != resync_id
        {
            state.pinned_snapshot = None;
            return Err(VoiceError::Unauthorized);
        }
        if cursor != transfer.next_cursor as u64 {
            state.pinned_snapshot = None;
            return Err(VoiceError::StaleCatalog);
        }
        let frame = if transfer.next_cursor < transfer.pages.len() {
            let page = transfer.pages[transfer.next_cursor].clone();
            transfer.next_cursor += 1;
            SnapshotFrame::Page(Box::new(page))
        } else {
            let end = transfer.end.clone();
            self.shared
                .events
                .acknowledge_snapshot(transfer.watermark, transfer.overflow_generation);
            state.pinned_snapshot = None;
            SnapshotFrame::End(end)
        };
        Ok(frame)
    }

    fn snapshot_header(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
    ) -> Result<SnapshotTransferHeader, VoiceError> {
        let SnapshotFrame::Begin(begin) = self.begin_snapshot(caller)? else {
            return Err(VoiceError::Internal);
        };
        Ok(SnapshotTransferHeader {
            resync_id: begin.resync_id,
            event_sequence: begin.event_sequence,
            revision: begin.revision,
            owner: begin.owner,
        })
    }

    fn command_result(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        accepted: bool,
        code: VoiceResultCode,
    ) -> Result<VoiceCommandWireResult, VoiceError> {
        Ok(VoiceCommandWireResult {
            accepted,
            code,
            transfer: Some(self.snapshot_header(caller)?),
        })
    }
    pub fn assets_get_status(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
    ) -> Result<AssetCatalog, VoiceError> {
        if !caller.is_trusted() {
            return Err(VoiceError::Unauthorized);
        }
        self.asset_manager
            .as_ref()
            .ok_or(VoiceError::SetupRequired)?
            .inventory()
            .map_err(map_asset_error)
    }

    fn asset_manager_for_change(
        &self,
        asset_id: &str,
    ) -> Result<Arc<dyn AssetManager>, VoiceError> {
        let manager = self
            .asset_manager
            .as_ref()
            .map(Arc::clone)
            .ok_or(VoiceError::SetupRequired)?;
        let catalog = manager.inventory().map_err(map_asset_error)?;
        if !catalog
            .assets
            .iter()
            .any(|asset| asset.asset_id == asset_id)
        {
            return Err(VoiceError::Unavailable);
        }
        Ok(manager)
    }

    fn stop_pipeline_for_asset_change(&self) -> Result<(), VoiceError> {
        let cleanup = self
            .stop_for_window_loss(NativeStopReason::AssetRevoked)
            .wait_for_cleanup(WORKER_REAP_DEADLINE + Duration::from_millis(250));
        if cleanup.verified {
            Ok(())
        } else {
            Err(VoiceError::CleanupFailed)
        }
    }

    pub fn assets_disable(&self, asset_id: &str) -> Result<AssetCatalog, VoiceError> {
        let manager = self.asset_manager_for_change(asset_id)?;
        self.stop_pipeline_for_asset_change()?;
        manager.disable(asset_id).map_err(map_asset_error)
    }

    pub fn assets_delete(&self, asset_id: &str) -> Result<AssetRemovalOutcome, VoiceError> {
        let manager = self.asset_manager_for_change(asset_id)?;
        self.stop_pipeline_for_asset_change()?;
        manager.delete(asset_id).map_err(map_asset_error)
    }

    pub fn assets_rollback(
        &self,
        asset_id: &str,
        version_id: &str,
    ) -> Result<AssetCatalog, VoiceError> {
        let manager = self.asset_manager_for_change(asset_id)?;
        self.stop_pipeline_for_asset_change()?;
        manager
            .rollback(asset_id, version_id)
            .map_err(map_asset_error)
    }
}
const WORKER_AUTHORIZATION_SESSION_LIFETIME: Duration = Duration::from_secs(5 * 60);
const WORKER_AUTHORIZATION_WORK_HEADROOM: Duration = Duration::from_secs(30 + 15 + 60);

fn cached_worker_authorization_session(
    cache: &Mutex<Option<Arc<crate::voice_authorization::AuthorizedWorkerSession>>>,
    authority: &dyn VoiceAssetAuthority,
    cancellation: &contract::CancelToken,
    required_until: Instant,
    needs: crate::voice_authorization::WorkerAuthorizationNeeds,
) -> Result<
    Arc<crate::voice_authorization::AuthorizedWorkerSession>,
    crate::voice_authorization::VoiceAuthorizationError,
> {
    if cancellation.is_cancelled() {
        return Err(crate::voice_authorization::VoiceAuthorizationError::OwnerCancelled);
    }
    {
        let mut cached = cache.lock();
        if let Some(session) = cached
            .as_ref()
            .filter(|session| session.is_usable(required_until, needs.asr, needs.llm))
        {
            return Ok(Arc::clone(session));
        }
        if let Some(session) = cached.take() {
            session.invalidate();
        }
    }
    let session_deadline = Instant::now() + WORKER_AUTHORIZATION_SESSION_LIFETIME;
    let session =
        authority.begin_worker_authorization_session(cancellation, session_deadline, needs)?;
    if cancellation.is_cancelled() {
        session.invalidate();
        return Err(crate::voice_authorization::VoiceAuthorizationError::OwnerCancelled);
    }
    *cache.lock() = Some(Arc::clone(&session));
    Ok(session)
}

fn map_worker_authorization_error(
    error: crate::voice_authorization::VoiceAuthorizationError,
) -> VoiceError {
    match error {
        crate::voice_authorization::VoiceAuthorizationError::OwnerCancelled => {
            VoiceError::Cancelled
        }
        crate::voice_authorization::VoiceAuthorizationError::OwnerAuthentication => {
            VoiceError::PermissionDenied
        }
        crate::voice_authorization::VoiceAuthorizationError::BindingUnfrozen
        | crate::voice_authorization::VoiceAuthorizationError::KeyUnavailable
        | crate::voice_authorization::VoiceAuthorizationError::RecoveryRequired => {
            VoiceError::SetupRequired
        }
        _ => VoiceError::Unavailable,
    }
}
fn invalidate_worker_authorization_session(shared: &VoiceShared) {
    if let Some(session) = shared.worker_session.lock().take() {
        session.invalidate();
    }
}

impl VoiceSessionSupervisor {
    fn begin_turn(
        &self,
        language_mode: LanguageMode,
        source: UtteranceSource,
        phase: VoicePhase,
        deadline: Duration,
    ) -> Result<TurnContext, VoiceError> {
        let mut state = self.shared.state.lock();
        if state.locked || state.shutting_down {
            return Err(VoiceError::CleanupFailed);
        }
        if state.active.is_some()
            || state.pending_review.is_some()
            || state.pending_intent.is_some()
        {
            return Err(VoiceError::Busy);
        }
        if state.snapshot.metadata.session_id == "session-unavailable" {
            return Err(VoiceError::Unavailable);
        }
        let generation = state
            .generation
            .checked_add(1)
            .ok_or(VoiceError::SequenceExhausted)?;
        let operation_id = state.next_operation_id;
        let next_operation_id = operation_id
            .checked_add(1)
            .ok_or(VoiceError::SequenceExhausted)?;
        let owner = VoiceOwner {
            session_id: state.snapshot.metadata.session_id.clone(),
            turn_id: format!("turn-{generation}"),
            generation,
        };
        let event_sender: Arc<dyn VoiceEventSender> = self.shared.events.clone();
        let context = TurnContext {
            owner: owner.clone(),
            operation_id: OperationId(operation_id),
            cancel: contract::CancelToken::default(),
            deadline: Instant::now() + deadline,
            event_sender,
        };
        state.generation = generation;
        state.next_operation_id = next_operation_id;
        state.active = Some(ActiveTurn {
            context: context.clone(),
            capture_operation_id: (source == UtteranceSource::Speech)
                .then_some(context.operation_id),
            language_mode,
            source,
        });
        state.pending_intent = None;
        state.snapshot.owner = Some(owner.clone());
        state.snapshot.pending.clear();
        state.snapshot.metadata.phase = phase;
        state.snapshot.metadata.preferences.language_mode = Some(language_mode);
        state.snapshot.metadata.cancel_available = true;
        state.snapshot.metadata.cleanup = CleanupReceipt::verified(CleanupCode::NoResourcesOwned);
        state.snapshot.metadata.route_kind = None;
        state.snapshot.metadata.capture_elapsed_ms = None;
        state.snapshot.metadata.error = None;
        state.snapshot.metadata.output_progress = OutputProgress {
            queued_segments: 0,
            completed_segments: 0,
            active: false,
        };
        let event = contract::PendingVoiceEvent {
            owner: Some(owner),
            operation_id: Some(context.operation_id),
            event: VoiceEventKind::StateChanged {
                phase,
                route_kind: None,
                cancel_available: true,
                cleanup: state.snapshot.metadata.cleanup,
            },
        };
        self.shared
            .events
            .try_send(event)
            .map_err(|_| VoiceError::Unavailable)?;
        self.shared.changed.notify_all();
        Ok(context)
    }

    fn next_context(
        shared: &VoiceShared,
        previous: &TurnContext,
        deadline: Duration,
    ) -> Result<TurnContext, VoiceError> {
        let mut state = shared.state.lock();
        let operation_id = state.next_operation_id;
        let next_operation_id = operation_id
            .checked_add(1)
            .ok_or(VoiceError::SequenceExhausted)?;
        let active = state
            .active
            .as_mut()
            .filter(|active| {
                active.context.owner == previous.owner && !previous.cancel.is_cancelled()
            })
            .ok_or(VoiceError::StaleOwner)?;
        let context = TurnContext {
            owner: previous.owner.clone(),
            operation_id: OperationId(operation_id),
            cancel: previous.cancel.clone(),
            deadline: Instant::now() + deadline,
            event_sender: previous.event_sender.clone(),
        };
        active.context = context.clone();
        state.next_operation_id = next_operation_id;
        Ok(context)
    }

    pub fn start(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        window: &tauri::WebviewWindow,
        request: StartRequest,
    ) -> Result<VoiceCommandWireResult, VoiceError> {
        self.authorize(&caller, &VoiceCommand::Start(request.clone()))?;
        caller.ensure_live(window)?;
        let device = {
            let state = self.shared.state.lock();
            let selected = request
                .input_device_handle
                .as_deref()
                .or(state
                    .snapshot
                    .metadata
                    .preferences
                    .input_device_handle
                    .as_deref())
                .map(str::to_owned)
                .or_else(|| {
                    state
                        .snapshot
                        .input_devices
                        .iter()
                        .find(|device| device.is_default)
                        .map(|device| device.handle.0.clone())
                })
                .ok_or(VoiceError::SetupRequired)?;
            state
                .snapshot
                .input_devices
                .iter()
                .find(|device| device.handle.0 == selected)
                .map(|device| device.handle.clone())
                .ok_or(VoiceError::StaleCatalog)?
        };
        let context = self.begin_turn(
            request.language_mode,
            UtteranceSource::Speech,
            VoicePhase::RequestingPermission,
            Duration::from_secs(u64::from(MAX_CAPTURE_SECONDS)),
        )?;
        let worker_authority: Arc<dyn VoiceAssetAuthority> = self.authorization.clone();
        let shared = Arc::clone(&self.shared);
        let capture = Arc::clone(&self.capture);
        let caller_for_worker = caller.clone();
        let window_for_worker = window.clone();
        let operation_context = context.clone();
        let spawn = thread::Builder::new()
            .name("saturday-voice-capture-start".to_owned())
            .spawn(move || {
                if caller_for_worker.ensure_live(&window_for_worker).is_err() {
                    operation_context.cancel.cancel();
                    finish_operation(
                        &shared,
                        &operation_context,
                        CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                        VoicePhase::Error,
                        Some(VoiceError::Unauthorized),
                    );
                    return;
                }
                let required_until =
                    operation_context.deadline + WORKER_AUTHORIZATION_WORK_HEADROOM;
                let needs = {
                    let state = shared.state.lock();
                    crate::voice_authorization::WorkerAuthorizationNeeds {
                        asr: true,
                        llm: state.snapshot.metadata.readiness.llm.state == ReadinessState::Ready,
                    }
                };
                let session = match cached_worker_authorization_session(
                    &shared.worker_session,
                    worker_authority.as_ref(),
                    &operation_context.cancel,
                    required_until,
                    needs,
                ) {
                    Ok(session) => session,
                    Err(error) => {
                        finish_operation(
                            &shared,
                            &operation_context,
                            CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                            VoicePhase::Error,
                            Some(map_worker_authorization_error(error)),
                        );
                        return;
                    }
                };
                if caller_for_worker.ensure_live(&window_for_worker).is_err() {
                    operation_context.cancel.cancel();
                    session.invalidate();
                    finish_operation(
                        &shared,
                        &operation_context,
                        CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                        VoicePhase::Error,
                        Some(VoiceError::Unauthorized),
                    );
                    return;
                }
                let result = capture
                    .lock()
                    .as_mut()
                    .ok_or(CaptureError::Unavailable)
                    .and_then(|provider| provider.start(&operation_context, device));
                match result {
                    Ok(_) => {
                        if operation_context.is_cancelled_or_expired() {
                            let receipt = capture
                                .lock()
                                .as_mut()
                                .map_or_else(CleanupReceipt::failed, |provider| {
                                    provider.cancel(&operation_context.owner)
                                });
                            finish_operation(
                                &shared,
                                &operation_context,
                                receipt,
                                VoicePhase::Error,
                                Some(VoiceError::Cancelled),
                            );
                        }
                    }
                    Err(error) => finish_operation(
                        &shared,
                        &operation_context,
                        CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                        VoicePhase::Error,
                        Some(map_capture_error(error)),
                    ),
                }
            });
        if spawn.is_err() {
            finish_operation(
                &self.shared,
                &context,
                CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                VoicePhase::Error,
                Some(VoiceError::Unavailable),
            );
            return Err(VoiceError::Unavailable);
        }
        self.command_result(caller, true, VoiceResultCode::Pending)
    }

    pub fn finish(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        window: &tauri::WebviewWindow,
        owner: VoiceOwner,
    ) -> Result<VoiceCommandWireResult, VoiceError> {
        self.authorize(
            &caller,
            &VoiceCommand::Finish(contract::OwnerRequest {
                owner: owner.clone(),
            }),
        )?;
        caller.ensure_live(window)?;
        let (context, capture_operation_id) = {
            let mut state = self.shared.state.lock();
            let (previous, capture_operation_id) = state
                .active
                .as_ref()
                .filter(|active| {
                    active.context.owner == owner && active.source == UtteranceSource::Speech
                })
                .map(|active| (active.context.clone(), active.capture_operation_id))
                .ok_or(VoiceError::StaleOwner)?;
            let capture_operation_id = capture_operation_id.ok_or(VoiceError::StaleOwner)?;
            if previous.is_cancelled_or_expired() {
                return Err(VoiceError::Cancelled);
            }
            if state.snapshot.metadata.phase != VoicePhase::Listening {
                return Err(VoiceError::Busy);
            }
            let operation_id = state.next_operation_id;
            let next_operation_id = operation_id
                .checked_add(1)
                .ok_or(VoiceError::SequenceExhausted)?;
            let context = TurnContext {
                owner: previous.owner.clone(),
                operation_id: OperationId(operation_id),
                cancel: previous.cancel.clone(),
                deadline: Instant::now() + ASR_DEADLINE,
                event_sender: previous.event_sender.clone(),
            };
            state.next_operation_id = next_operation_id;
            if let Some(active) = state.active.as_mut() {
                active.context = context.clone();
            }
            state.snapshot.metadata.capture_elapsed_ms = None;
            let _ = self.shared.events.try_send(contract::PendingVoiceEvent {
                owner: Some(owner.clone()),
                operation_id: Some(context.operation_id),
                event: VoiceEventKind::StateChanged {
                    phase: VoicePhase::Transcribing,
                    route_kind: None,
                    cancel_available: true,
                    cleanup: state.snapshot.metadata.cleanup,
                },
            });
            self.shared.changed.notify_all();
            (context, capture_operation_id)
        };
        let shared = Arc::clone(&self.shared);
        let capture = Arc::clone(&self.capture);
        let asr = Arc::clone(&self.asr);
        let speech = Arc::clone(&self.speech);
        let assets = self.asset_manager.clone();
        let caller_for_worker = caller.clone();
        let window_for_worker = window.clone();
        let context_for_worker = context.clone();
        let spawn = thread::Builder::new()
            .name("saturday-voice-transcribe".to_owned())
            .spawn(move || {
                let context = context_for_worker;
                if caller_for_worker.ensure_live(&window_for_worker).is_err() {
                    context.cancel.cancel();
                    let receipt = capture
                        .lock()
                        .as_mut()
                        .map_or_else(CleanupReceipt::failed, |provider| provider.cancel(&owner));
                    finish_operation(
                        &shared,
                        &context,
                        receipt,
                        VoicePhase::Error,
                        Some(VoiceError::Unauthorized),
                    );
                    return;
                }
                let capture_result = {
                    let mut provider = capture.lock();
                    let result = provider
                        .as_mut()
                        .ok_or(CaptureError::Unavailable)
                        .and_then(|provider| provider.finish(&owner));
                    if result.is_ok() {
                        let mut state = shared.state.lock();
                        if let Some(active) = state.active.as_mut().filter(|active| {
                            active.context.owner == owner
                                && active.capture_operation_id == Some(capture_operation_id)
                        }) {
                            active.capture_operation_id = None;
                        }
                    }
                    result
                };
                let pcm = match capture_result {
                    Ok(pcm) => pcm,
                    Err(error) => {
                        finish_operation(
                            &shared,
                            &context,
                            CleanupReceipt::failed(),
                            VoicePhase::Error,
                            Some(map_capture_error(error)),
                        );
                        return;
                    }
                };
                if context.is_cancelled_or_expired() {
                    finish_operation(
                        &shared,
                        &context,
                        CleanupReceipt::verified(CleanupCode::CaptureStopped),
                        VoicePhase::Error,
                        Some(VoiceError::Cancelled),
                    );
                    return;
                }
                if caller_for_worker.ensure_live(&window_for_worker).is_err() {
                    context.cancel.cancel();
                    finish_operation(
                        &shared,
                        &context,
                        CleanupReceipt::verified(CleanupCode::CaptureStopped),
                        VoicePhase::Error,
                        Some(VoiceError::Unauthorized),
                    );
                    return;
                }
                let Some(assets) = assets else {
                    finish_operation(
                        &shared,
                        &context,
                        CleanupReceipt::verified(CleanupCode::CaptureStopped),
                        VoicePhase::SetupRequired,
                        Some(VoiceError::SetupRequired),
                    );
                    return;
                };
                let model = match assets.open_model("whisper-base", AssetPurpose::AsrWeights) {
                    Ok(model) => model,
                    Err(error) => {
                        finish_operation(
                            &shared,
                            &context,
                            CleanupReceipt::verified(CleanupCode::CaptureStopped),
                            VoicePhase::SetupRequired,
                            Some(map_asset_error(error)),
                        );
                        return;
                    }
                };
                if caller_for_worker.ensure_live(&window_for_worker).is_err() {
                    context.cancel.cancel();
                    finish_operation(
                        &shared,
                        &context,
                        CleanupReceipt::verified(CleanupCode::CaptureStopped),
                        VoicePhase::Error,
                        Some(VoiceError::Unauthorized),
                    );
                    return;
                }
                let asr_context = context.clone();
                let language_mode = {
                    let state = shared.state.lock();
                    state
                        .active
                        .as_ref()
                        .filter(|active| active.context.owner == owner)
                        .map(|active| active.language_mode)
                        .unwrap_or(LanguageMode::Auto)
                };
                let worker_session = shared.worker_session.lock().as_ref().cloned();
                let Some(worker_session) = worker_session.filter(|session| {
                    session.is_usable(asr_context.deadline + GENERATION_DEADLINE, true, false)
                }) else {
                    finish_operation(
                        &shared,
                        &asr_context,
                        CleanupReceipt::verified(CleanupCode::CaptureStopped),
                        VoicePhase::SetupRequired,
                        Some(VoiceError::SetupRequired),
                    );
                    return;
                };
                let result = asr
                    .lock()
                    .as_mut()
                    .ok_or(contract::AsrError::AssetUnavailable)
                    .and_then(|provider| {
                        provider.transcribe(
                            &asr_context,
                            pcm,
                            language_mode,
                            model,
                            Arc::clone(&worker_session),
                        )
                    });
                match result {
                    Ok(result) => accept_asr_result(
                        &shared,
                        &asr_context,
                        result,
                        Some(assets),
                        speech,
                        caller_for_worker,
                        window_for_worker,
                    ),
                    Err(error) => finish_operation(
                        &shared,
                        &asr_context,
                        CleanupReceipt::verified(CleanupCode::CaptureStopped),
                        VoicePhase::Error,
                        Some(map_asr_error(error)),
                    ),
                }
            });
        if spawn.is_err() {
            context.cancel.cancel();
            finish_operation(
                &self.shared,
                &context,
                CleanupReceipt::failed(),
                VoicePhase::Error,
                Some(VoiceError::Unavailable),
            );
            return Err(VoiceError::Unavailable);
        }
        self.command_result(caller, true, VoiceResultCode::Pending)
    }
}

fn finish_operation(
    shared: &VoiceShared,
    context: &TurnContext,
    receipt: CleanupReceipt,
    phase: VoicePhase,
    error: Option<VoiceError>,
) {
    let mut state = shared.state.lock();
    let matches_active = state.active.as_ref().is_some_and(|active| {
        active.context.owner == context.owner && active.context.operation_id == context.operation_id
    });
    if !matches_active || state.snapshot.owner.as_ref() != Some(&context.owner) {
        return;
    }
    if !receipt.verified {
        state.locked = true;
    }
    if receipt.verified
        || state
            .active
            .as_ref()
            .is_none_or(|active| active.capture_operation_id.is_none())
    {
        state.active = None;
    }
    state.snapshot.metadata.phase = if receipt.verified {
        phase
    } else {
        VoicePhase::Error
    };
    state.snapshot.metadata.cancel_available = false;
    state.snapshot.metadata.cleanup = receipt;
    state.snapshot.metadata.error = if receipt.verified {
        error
    } else {
        Some(VoiceError::CleanupFailed)
    };
    if let Some(code) = state.snapshot.metadata.error {
        let _ = shared.events.try_send(contract::PendingVoiceEvent {
            owner: Some(context.owner.clone()),
            operation_id: Some(context.operation_id),
            event: VoiceEventKind::Failure { code },
        });
    }
    let _ = shared.events.try_send(contract::PendingVoiceEvent {
        owner: Some(context.owner.clone()),
        operation_id: Some(context.operation_id),
        event: VoiceEventKind::StateChanged {
            phase: state.snapshot.metadata.phase,
            route_kind: state.snapshot.metadata.route_kind,
            cancel_available: false,
            cleanup: receipt,
        },
    });
    let _ = shared.events.try_send(contract::PendingVoiceEvent {
        owner: Some(context.owner.clone()),
        operation_id: Some(context.operation_id),
        event: VoiceEventKind::Terminal { receipt },
    });
    shared.changed.notify_all();
}

fn spawn_event_monitor(
    shared: Weak<VoiceShared>,
    capture: Arc<Mutex<Option<crate::voice::capture::NativeAudioCaptureProvider>>>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("saturday-voice-events".to_owned())
        .spawn(move || {
            let mut cursor = 0u64;
            loop {
                let Some(shared) = shared.upgrade() else {
                    break;
                };
                if shared.monitor_shutdown.load(Ordering::Acquire) {
                    break;
                }
                let events = shared.events.wait_after(cursor, Duration::from_millis(50));
                for event in events {
                    cursor = cursor.max(event.wire.event_sequence.0);
                    apply_native_event(&shared, &capture, &event);
                }
            }
        })
        .expect("voice event monitor thread must be available")
}

fn apply_device_loss_event(
    state: &Mutex<SessionState>,
    events: &BoundedVoiceEventQueue,
    changed: &Condvar,
    owner: &VoiceOwner,
    operation_id: OperationId,
    cancel_capture: impl FnOnce(&VoiceOwner, OperationId) -> CleanupReceipt,
) {
    let cancellation = {
        let state = state.lock();
        if state.snapshot.owner.as_ref() != Some(owner) {
            return;
        }
        let Some(active) = state.active.as_ref().filter(|active| {
            active.context.owner == *owner
                && (active.context.operation_id == operation_id
                    || active.capture_operation_id == Some(operation_id))
        }) else {
            return;
        };
        active.context.cancel.clone()
    };
    cancellation.cancel();

    let receipt = cancel_capture(owner, operation_id);
    let mut state = state.lock();
    let still_matches_active = state.snapshot.owner.as_ref() == Some(owner)
        && state.active.as_ref().is_some_and(|active| {
            active.context.owner == *owner
                && (active.context.operation_id == operation_id
                    || active.capture_operation_id == Some(operation_id))
        });
    if !still_matches_active {
        changed.notify_all();
        return;
    }

    state.snapshot.metadata.phase = VoicePhase::Error;
    state.snapshot.metadata.cancel_available = false;
    state.snapshot.metadata.cleanup = receipt;
    state.snapshot.metadata.error = Some(if receipt.verified {
        VoiceError::Unavailable
    } else {
        state.locked = true;
        VoiceError::CleanupFailed
    });
    if receipt.verified {
        state.active = None;
        state.locked = false;
    }
    let _ = events.try_send(contract::PendingVoiceEvent {
        owner: Some(owner.clone()),
        operation_id: Some(operation_id),
        event: VoiceEventKind::StateChanged {
            phase: VoicePhase::Error,
            route_kind: state.snapshot.metadata.route_kind,
            cancel_available: state.snapshot.metadata.cancel_available,
            cleanup: receipt,
        },
    });
    changed.notify_all();
}

fn apply_native_event(
    shared: &VoiceShared,
    capture: &Mutex<Option<crate::voice::capture::NativeAudioCaptureProvider>>,
    queued: &QueuedVoiceEvent,
) {
    let wire = &queued.wire;
    if matches!(
        &wire.event,
        VoiceEventKind::CaptureLifecycle {
            state: CaptureLifecycleState::DeviceLost
        }
    ) {
        {
            let mut state = shared.state.lock();
            state.applied_event_sequence = wire.event_sequence.0;
            state.applied_revision = wire.revision.0;
        }
        if let (Some(owner), Some(operation_id)) = (wire.owner.as_ref(), queued.operation_id) {
            apply_device_loss_event(
                &shared.state,
                &shared.events,
                &shared.changed,
                owner,
                operation_id,
                |owner, operation_id| {
                    capture
                        .lock()
                        .as_mut()
                        .map_or_else(CleanupReceipt::failed, |provider| {
                            provider.cancel_operation(owner, operation_id.0)
                        })
                },
            );
        } else {
            shared.changed.notify_all();
        }
        return;
    }

    let mut state = shared.state.lock();
    state.applied_event_sequence = wire.event_sequence.0;
    state.applied_revision = wire.revision.0;
    let Some(owner) = wire.owner.as_ref() else {
        shared.changed.notify_all();
        return;
    };
    if state.snapshot.owner.as_ref() != Some(owner) {
        shared.changed.notify_all();
        return;
    }
    let operation = state
        .active
        .as_ref()
        .filter(|active| {
            active.context.owner == *owner
                && Some(active.context.operation_id) == queued.operation_id
        })
        .map(|active| active.context.clone());
    match &wire.event {
        VoiceEventKind::CaptureLifecycle { state: lifecycle } => {
            let Some(context) = operation else {
                return;
            };
            let phase = match lifecycle {
                CaptureLifecycleState::PermissionPending => Some(VoicePhase::RequestingPermission),
                CaptureLifecycleState::Started
                    if state.snapshot.metadata.phase == VoicePhase::RequestingPermission =>
                {
                    Some(VoicePhase::Listening)
                }
                CaptureLifecycleState::PermissionDenied | CaptureLifecycleState::Failed => {
                    Some(VoicePhase::Error)
                }
                CaptureLifecycleState::DeviceLost => None,
                CaptureLifecycleState::Cancelled => None,
                CaptureLifecycleState::Started => None,
            };
            if let Some(phase) = phase {
                state.snapshot.metadata.phase = phase;
                if matches!(
                    lifecycle,
                    CaptureLifecycleState::PermissionDenied | CaptureLifecycleState::Failed
                ) {
                    let code = if *lifecycle == CaptureLifecycleState::PermissionDenied {
                        VoiceError::PermissionDenied
                    } else {
                        VoiceError::ProviderFailed
                    };
                    state.snapshot.metadata.error = Some(code);
                    state.snapshot.metadata.cancel_available = false;
                    state.active = None;
                    state.snapshot.metadata.cleanup = CleanupReceipt::failed();
                    state.locked = true;
                }
                let _ = shared.events.try_send(contract::PendingVoiceEvent {
                    owner: Some(owner.clone()),
                    operation_id: Some(context.operation_id),
                    event: VoiceEventKind::StateChanged {
                        phase,
                        route_kind: state.snapshot.metadata.route_kind,
                        cancel_available: state.snapshot.metadata.cancel_available,
                        cleanup: state.snapshot.metadata.cleanup,
                    },
                });
            }
        }
        VoiceEventKind::CaptureMeter { elapsed_ms, .. } => {
            if state.snapshot.metadata.phase == VoicePhase::Listening {
                state.snapshot.metadata.capture_elapsed_ms = Some(*elapsed_ms);
            }
        }
        VoiceEventKind::OutputStarted => {
            state.snapshot.metadata.phase = VoicePhase::Speaking;
            state.snapshot.metadata.output_progress.active = true;
        }
        VoiceEventKind::Terminal { receipt } => {
            state.snapshot.metadata.cleanup = *receipt;
            state.snapshot.metadata.output_progress.completed_segments = state
                .snapshot
                .metadata
                .output_progress
                .completed_segments
                .saturating_add(1);
            if !receipt.verified {
                state.locked = true;
                state.snapshot.metadata.phase = VoicePhase::Error;
                state.snapshot.metadata.error = Some(VoiceError::CleanupFailed);
                state.snapshot.metadata.cancel_available = false;
            }
        }
        VoiceEventKind::OutputDrained => {
            state.snapshot.metadata.output_progress.active = false;
            state.snapshot.metadata.output_progress.queued_segments = 0;
            state.snapshot.metadata.phase = VoicePhase::Idle;
            state.snapshot.metadata.cancel_available = false;
            state.active = None;
            state.snapshot.metadata.cleanup = CleanupReceipt::verified(CleanupCode::SpeechStopped);
        }
        VoiceEventKind::Failure { code } => {
            state.snapshot.metadata.error = Some(*code);
            if *code == VoiceError::CleanupFailed {
                state.locked = true;
            }
        }
        _ => {}
    }
    shared.changed.notify_all();
}

fn map_asset_error(error: AssetError) -> VoiceError {
    match error {
        AssetError::NotFound => VoiceError::Unavailable,
        AssetError::WrongPurpose => VoiceError::Unauthorized,
        AssetError::Unavailable => VoiceError::Unavailable,
        AssetError::ApprovalRequired => VoiceError::SetupRequired,
        AssetError::InvalidBinding | AssetError::DigestMismatch => VoiceError::ProtocolError,
        AssetError::StorageFailure => VoiceError::Internal,
        AssetError::Cancelled => VoiceError::Cancelled,
        AssetError::Timeout => VoiceError::Timeout,
        AssetError::CleanupFailed => VoiceError::CleanupFailed,
        AssetError::Failed => VoiceError::ProviderFailed,
    }
}

fn map_capture_error(error: CaptureError) -> VoiceError {
    match error {
        CaptureError::Unavailable => VoiceError::Unavailable,
        CaptureError::PermissionDenied => VoiceError::PermissionDenied,
        CaptureError::DeviceLost => VoiceError::Unavailable,
        CaptureError::UnsupportedFormat => VoiceError::Unsupported,
        CaptureError::Overflow => VoiceError::TooLarge,
        CaptureError::InvalidAudio => VoiceError::InvalidRequest,
        CaptureError::Cancelled => VoiceError::Cancelled,
        CaptureError::Timeout => VoiceError::Timeout,
        CaptureError::CleanupFailed => VoiceError::CleanupFailed,
        CaptureError::Failed => VoiceError::ProviderFailed,
    }
}

fn map_asr_error(error: contract::AsrError) -> VoiceError {
    match error {
        contract::AsrError::Unavailable => VoiceError::Unavailable,
        contract::AsrError::AssetUnavailable => VoiceError::SetupRequired,
        contract::AsrError::InvalidAudio | contract::AsrError::InvalidEvidence => {
            VoiceError::InvalidRequest
        }
        contract::AsrError::NoSpeech => VoiceError::Cancelled,
        contract::AsrError::UnsupportedLanguage => VoiceError::Unsupported,
        contract::AsrError::Timeout => VoiceError::Timeout,
        contract::AsrError::Cancelled => VoiceError::Cancelled,
        contract::AsrError::WorkerFailed | contract::AsrError::Failed => VoiceError::ProviderFailed,
        contract::AsrError::CleanupFailed => VoiceError::CleanupFailed,
    }
}

impl VoiceSessionSupervisor {
    pub fn submit_text(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        window: &tauri::WebviewWindow,
        request: SubmitTextRequest,
    ) -> Result<VoiceCommandWireResult, VoiceError> {
        self.authorize(&caller, &VoiceCommand::SubmitText(request.clone()))?;
        caller.ensure_live(window)?;
        contract::validate_typed_request(&request.text).map_err(|error| match error {
            contract::RequestLimitError::TooLong => VoiceError::TooLarge,
            contract::RequestLimitError::ReviewRequired => VoiceError::InvalidRequest,
        })?;
        let language = typed_language(&request.text, request.language_mode);
        let context = self.begin_turn(
            request.language_mode,
            UtteranceSource::Typed,
            VoicePhase::Routing,
            GENERATION_DEADLINE,
        )?;
        let utterance = AcceptedUtterance {
            text: request.text,
            language,
            source: UtteranceSource::Typed,
            eligibility: TranscriptEligibility::Eligible,
        };
        let shared = Arc::clone(&self.shared);
        let assets = self.asset_manager.clone();
        let speech = Arc::clone(&self.speech);
        let caller_for_worker = caller.clone();
        let window_for_worker = window.clone();
        let operation_context = context.clone();
        let spawn = thread::Builder::new()
            .name("saturday-voice-text-route".to_owned())
            .spawn(move || {
                if caller_for_worker.ensure_live(&window_for_worker).is_err() {
                    finish_operation(
                        &shared,
                        &operation_context,
                        CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                        VoicePhase::Error,
                        Some(VoiceError::Unauthorized),
                    );
                    return;
                }
                route_accepted(RouteRequest {
                    shared,
                    assets,
                    speech,
                    caller: caller_for_worker,
                    window: window_for_worker,
                    context: operation_context,
                    utterance,
                    record_history: false,
                });
            });
        if spawn.is_err() {
            finish_operation(
                &self.shared,
                &context,
                CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                VoicePhase::Error,
                Some(VoiceError::Unavailable),
            );
            return Err(VoiceError::Unavailable);
        }
        self.command_result(caller, true, VoiceResultCode::Pending)
    }

    pub fn confirm_transcript(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        window: &tauri::WebviewWindow,
        request: ConfirmTranscriptRequest,
    ) -> Result<VoiceCommandWireResult, VoiceError> {
        self.authorize(&caller, &VoiceCommand::ConfirmTranscript(request.clone()))?;
        caller.ensure_live(window)?;
        contract::validate_spoken_request(&request.text).map_err(|error| match error {
            contract::RequestLimitError::TooLong => VoiceError::TooLarge,
            contract::RequestLimitError::ReviewRequired => VoiceError::InvalidRequest,
        })?;
        let context = {
            let mut state = self.shared.state.lock();
            let review = state
                .pending_review
                .as_ref()
                .filter(|review| review.owner == request.owner)
                .ok_or(VoiceError::StaleOwner)?;
            if review.text.is_empty() {
                return Err(VoiceError::InvalidRequest);
            }
            let operation_id = state.next_operation_id;
            state.next_operation_id = operation_id
                .checked_add(1)
                .ok_or(VoiceError::SequenceExhausted)?;
            let event_sender: Arc<dyn VoiceEventSender> = self.shared.events.clone();
            let context = TurnContext {
                owner: request.owner.clone(),
                operation_id: OperationId(operation_id),
                cancel: contract::CancelToken::default(),
                deadline: Instant::now() + GENERATION_DEADLINE,
                event_sender,
            };
            state.pending_review = None;
            state.active = Some(ActiveTurn {
                context: context.clone(),
                capture_operation_id: None,
                language_mode: LanguageMode::from(request.language),
                source: UtteranceSource::Speech,
            });
            state.snapshot.metadata.phase = VoicePhase::Routing;
            state.snapshot.metadata.cancel_available = true;
            state.snapshot.metadata.error = None;
            state.snapshot.pending.clear();
            context
        };
        let utterance = AcceptedUtterance {
            text: request.text,
            language: request.language,
            source: UtteranceSource::Speech,
            eligibility: TranscriptEligibility::Eligible,
        };
        if publish_transcript(&self.shared, &context, &utterance, true).is_err() {
            finish_operation(
                &self.shared,
                &context,
                CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                VoicePhase::Error,
                Some(VoiceError::Unavailable),
            );
            return Err(VoiceError::Unavailable);
        }
        let shared = Arc::clone(&self.shared);
        let assets = self.asset_manager.clone();
        let speech = Arc::clone(&self.speech);
        let caller_for_worker = caller.clone();
        let window_for_worker = window.clone();
        let operation_context = context.clone();
        let spawn = thread::Builder::new()
            .name("saturday-voice-confirm-route".to_owned())
            .spawn(move || {
                route_accepted(RouteRequest {
                    shared,
                    assets,
                    speech,
                    caller: caller_for_worker,
                    window: window_for_worker,
                    context: operation_context,
                    utterance,
                    record_history: true,
                });
            });
        if spawn.is_err() {
            finish_operation(
                &self.shared,
                &context,
                CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                VoicePhase::Error,
                Some(VoiceError::Unavailable),
            );
            return Err(VoiceError::Unavailable);
        }
        self.command_result(caller, true, VoiceResultCode::Pending)
    }
}

fn typed_language(text: &str, mode: LanguageMode) -> Language {
    match mode {
        LanguageMode::Tr => Language::Tr,
        LanguageMode::En => Language::En,
        LanguageMode::Auto => {
            let lower = text.to_lowercase();
            if text.chars().any(|character| "çğıöşüİı".contains(character))
                || [
                    "merhaba",
                    "nasılsın",
                    "teşekkür",
                    "lütfen",
                    "günaydın",
                    "istanbul",
                    "türkçe",
                    "yardım",
                    "durum",
                ]
                .iter()
                .any(|word| lower.contains(word))
            {
                Language::Tr
            } else {
                Language::En
            }
        }
    }
}

fn publish_transcript(
    shared: &VoiceShared,
    context: &TurnContext,
    utterance: &AcceptedUtterance,
    final_chunk: bool,
) -> Result<(), VoiceError> {
    if utterance.text.len() > MAX_SPOKEN_BYTES
        || utterance.text.chars().count() > MAX_SPOKEN_SCALARS
    {
        return Err(VoiceError::TooLarge);
    }
    let mut state = shared.state.lock();
    if state.snapshot.owner.as_ref() != Some(&context.owner)
        || !state.active.as_ref().is_some_and(|active| {
            active.context.owner == context.owner
                && active.context.operation_id == context.operation_id
        })
    {
        return Err(VoiceError::StaleOwner);
    }
    let sequence = state.snapshot.metadata.next_transcript_sequence;
    let following = sequence
        .checked_add(1)
        .ok_or(VoiceError::SequenceExhausted)?;
    state.snapshot.metadata.next_transcript_sequence = following;
    state
        .snapshot
        .pending
        .retain(|item| item.stream != PendingStream::Transcript);
    state.snapshot.pending.push(PendingText {
        stream: PendingStream::Transcript,
        sequence,
        language: utterance.language,
        source: utterance.source,
        text: utterance.text.clone(),
        final_chunk,
        partial: utterance.eligibility != TranscriptEligibility::Eligible,
    });
    shared
        .events
        .try_send(contract::PendingVoiceEvent {
            owner: Some(context.owner.clone()),
            operation_id: Some(context.operation_id),
            event: VoiceEventKind::TranscriptDelta {
                sequence,
                text: utterance.text.clone(),
                final_chunk,
                language: Some(utterance.language),
                source: utterance.source,
            },
        })
        .map_err(|_| VoiceError::Unavailable)
}

fn publish_answer_delta(
    shared: &VoiceShared,
    context: &TurnContext,
    language: Language,
    text: &str,
    final_chunk: bool,
    partial: bool,
) -> Result<(), VoiceError> {
    if text.len() > MAX_TEXT_DELTA_BYTES {
        return Err(VoiceError::TooLarge);
    }
    let mut state = shared.state.lock();
    if state.snapshot.owner.as_ref() != Some(&context.owner)
        || !state.active.as_ref().is_some_and(|active| {
            active.context.owner == context.owner
                && active.context.operation_id == context.operation_id
        })
    {
        return Err(VoiceError::StaleOwner);
    }
    let sequence = state.snapshot.metadata.next_answer_sequence;
    let following = sequence
        .checked_add(1)
        .ok_or(VoiceError::SequenceExhausted)?;
    let pending = state
        .snapshot
        .pending
        .iter_mut()
        .find(|pending| pending.stream == PendingStream::Answer);
    if let Some(pending) = pending {
        let new_len = pending
            .text
            .len()
            .checked_add(text.len())
            .ok_or(VoiceError::TooLarge)?;
        if new_len > MAX_MODEL_OUTPUT_BYTES {
            return Err(VoiceError::TooLarge);
        }
        pending.text.push_str(text);
        pending.sequence = sequence;
        pending.final_chunk = final_chunk;
        pending.partial = partial;
    } else {
        let source = state
            .active
            .as_ref()
            .map(|active| active.source)
            .unwrap_or(UtteranceSource::Typed);
        state.snapshot.pending.push(PendingText {
            stream: PendingStream::Answer,
            sequence,
            language,
            source,
            text: text.to_owned(),
            final_chunk,
            partial,
        });
    }
    state.snapshot.metadata.next_answer_sequence = following;
    shared
        .events
        .try_send(contract::PendingVoiceEvent {
            owner: Some(context.owner.clone()),
            operation_id: Some(context.operation_id),
            event: VoiceEventKind::AnswerDelta {
                sequence,
                text: text.to_owned(),
                final_chunk,
                language,
                partial,
            },
        })
        .map_err(|_| VoiceError::Unavailable)
}

fn commit_completed_turn(
    state: &mut SessionState,
    context: &TurnContext,
    utterance: &AcceptedUtterance,
    answer: &str,
    answer_language: Language,
) -> Result<(), VoiceError> {
    if utterance.text.is_empty() || answer.is_empty() {
        return Err(VoiceError::InvalidRequest);
    }
    state
        .history
        .commit(crate::voice::context::CompletedTurn::new(
            context.owner.turn_id.clone(),
            utterance.language,
            answer_language,
            utterance.text.clone(),
            answer.to_owned(),
        ))
        .map_err(|_| VoiceError::TooLarge)?;
    state.snapshot.history = state
        .history
        .completed()
        .iter()
        .flat_map(|turn| {
            [
                contract::HistoryMessage {
                    id: format!("{}:user", turn.turn_id),
                    role: MessageRole::User,
                    language: turn.user_language,
                    source: utterance.source,
                    text: turn.user_text.clone(),
                },
                contract::HistoryMessage {
                    id: format!("{}:assistant", turn.turn_id),
                    role: MessageRole::Assistant,
                    language: turn.assistant_language,
                    source: utterance.source,
                    text: turn.assistant_text.clone(),
                },
            ]
        })
        .collect();
    state.snapshot.pending.clear();
    Ok(())
}

fn set_phase(
    shared: &VoiceShared,
    context: &TurnContext,
    phase: VoicePhase,
    route_kind: Option<RouteKind>,
) -> Result<(), VoiceError> {
    let mut state = shared.state.lock();
    if state.snapshot.owner.as_ref() != Some(&context.owner)
        || !state.active.as_ref().is_some_and(|active| {
            active.context.owner == context.owner
                && active.context.operation_id == context.operation_id
        })
    {
        return Err(VoiceError::StaleOwner);
    }
    state.snapshot.metadata.phase = phase;
    state.snapshot.metadata.route_kind = route_kind;
    shared
        .events
        .try_send(contract::PendingVoiceEvent {
            owner: Some(context.owner.clone()),
            operation_id: Some(context.operation_id),
            event: VoiceEventKind::StateChanged {
                phase,
                route_kind,
                cancel_available: state.snapshot.metadata.cancel_available,
                cleanup: state.snapshot.metadata.cleanup,
            },
        })
        .map_err(|_| VoiceError::Unavailable)?;
    shared.changed.notify_all();
    Ok(())
}

fn accept_asr_result(
    shared: &Arc<VoiceShared>,
    context: &TurnContext,
    result: contract::AsrResult,
    assets: Option<Arc<dyn AssetManager>>,
    speech: Arc<Mutex<Option<crate::voice::speech::SystemSpeechProvider>>>,
    caller: crate::voice::ipc::NativeCallerContext,
    window: tauri::WebviewWindow,
) {
    if context.is_cancelled_or_expired() {
        finish_operation(
            shared,
            context,
            CleanupReceipt::verified(CleanupCode::CaptureStopped),
            VoicePhase::Error,
            Some(VoiceError::Cancelled),
        );
        return;
    }
    if result.eligibility == TranscriptEligibility::NoSpeech {
        {
            let mut state = shared.state.lock();
            state.snapshot.metadata.notices.push(NoticeCode::NoSpeech);
        }
        finish_operation(
            shared,
            context,
            CleanupReceipt::verified(CleanupCode::CaptureStopped),
            VoicePhase::Idle,
            None,
        );
        return;
    }
    let language = match result
        .ranked_languages
        .first()
        .map(|evidence| &evidence.language)
    {
        Some(contract::AsrLanguage::Tr) => Language::Tr,
        Some(contract::AsrLanguage::En) => Language::En,
        _ => {
            finish_operation(
                shared,
                context,
                CleanupReceipt::verified(CleanupCode::CaptureStopped),
                VoicePhase::Error,
                Some(VoiceError::Unsupported),
            );
            return;
        }
    };
    let utterance = AcceptedUtterance {
        text: result.text,
        language,
        source: UtteranceSource::Speech,
        eligibility: result.eligibility,
    };
    if contract::validate_spoken_request(&utterance.text).is_err() {
        finish_operation(
            shared,
            context,
            CleanupReceipt::verified(CleanupCode::CaptureStopped),
            VoicePhase::Error,
            Some(VoiceError::TooLarge),
        );
        return;
    }
    if publish_transcript(shared, context, &utterance, true).is_err() {
        finish_operation(
            shared,
            context,
            CleanupReceipt::verified(CleanupCode::CaptureStopped),
            VoicePhase::Error,
            Some(VoiceError::Unavailable),
        );
        return;
    }
    let review_required = utterance.eligibility != TranscriptEligibility::Eligible
        || shared
            .state
            .lock()
            .snapshot
            .metadata
            .preferences
            .review_before_send;
    if review_required {
        let mut state = shared.state.lock();
        state.pending_review = Some(PendingReview {
            owner: context.owner.clone(),
            text: utterance.text.clone(),
        });
        state.active = None;
        state.snapshot.metadata.phase = VoicePhase::NeedsReview;
        state.snapshot.metadata.cancel_available = false;
        state.snapshot.metadata.cleanup = CleanupReceipt::verified(CleanupCode::CaptureStopped);
        if utterance.eligibility != TranscriptEligibility::Eligible {
            state
                .snapshot
                .metadata
                .notices
                .push(NoticeCode::TranscriptNeedsReview);
        }
        let _ = shared.events.try_send(contract::PendingVoiceEvent {
            owner: Some(context.owner.clone()),
            operation_id: Some(context.operation_id),
            event: VoiceEventKind::StateChanged {
                phase: VoicePhase::NeedsReview,
                route_kind: None,
                cancel_available: false,
                cleanup: state.snapshot.metadata.cleanup,
            },
        });
        let _ = shared.events.try_send(contract::PendingVoiceEvent {
            owner: Some(context.owner.clone()),
            operation_id: Some(context.operation_id),
            event: VoiceEventKind::Terminal {
                receipt: state.snapshot.metadata.cleanup,
            },
        });
        shared.changed.notify_all();
        return;
    }
    route_accepted(RouteRequest {
        shared: Arc::clone(shared),
        assets,
        speech,
        caller,
        window,
        context: context.clone(),
        utterance,
        record_history: true,
    });
}

struct RouteRequest {
    shared: Arc<VoiceShared>,
    assets: Option<Arc<dyn AssetManager>>,
    speech: Arc<Mutex<Option<crate::voice::speech::SystemSpeechProvider>>>,
    caller: crate::voice::ipc::NativeCallerContext,
    window: tauri::WebviewWindow,
    context: TurnContext,
    utterance: AcceptedUtterance,
    record_history: bool,
}

fn route_accepted(request: RouteRequest) {
    let RouteRequest {
        shared,
        assets,
        speech,
        caller,
        window,
        context,
        utterance,
        record_history,
    } = request;
    if context.is_cancelled_or_expired() {
        finish_operation(
            &shared,
            &context,
            CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
            VoicePhase::Error,
            Some(VoiceError::Cancelled),
        );
        return;
    }
    if !record_history && publish_transcript(&shared, &context, &utterance, true).is_err() {
        finish_operation(
            &shared,
            &context,
            CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
            VoicePhase::Error,
            Some(VoiceError::Unavailable),
        );
        return;
    }
    let readiness = shared.state.lock().snapshot.metadata.readiness.clone();
    let decision = crate::voice::router::DeterministicIntentRouter.route(
        &utterance,
        &readiness,
        &context.owner,
        context.operation_id,
    );
    match decision {
        RouterDecision::LanguagePreference { language, response } => {
            let answer_context = match VoiceSessionSupervisor::next_context(
                &shared,
                &context,
                GENERATION_DEADLINE,
            ) {
                Ok(context) => context,
                Err(error) => {
                    finish_operation(
                        &shared,
                        &context,
                        CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                        VoicePhase::Error,
                        Some(error),
                    );
                    return;
                }
            };
            let _ = set_phase(
                &shared,
                &answer_context,
                VoicePhase::Routing,
                Some(response.kind),
            );
            {
                let mut state = shared.state.lock();
                state.snapshot.metadata.preferences.language_mode = Some(language.into());
            }
            publish_answer_and_finish(AnswerPublication {
                shared,
                speech,
                context: answer_context,
                utterance,
                answer: response.text,
                answer_language: response.language,
                route_kind: response.kind,
                record_history,
            });
        }
        RouterDecision::Resolved(response) => {
            if response
                .intent
                .as_ref()
                .is_some_and(|descriptor| descriptor_owner(descriptor) != &context.owner)
            {
                finish_operation(
                    &shared,
                    &context,
                    CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                    VoicePhase::Error,
                    Some(VoiceError::StaleOwner),
                );
                return;
            }
            if let Some(descriptor) = response.intent {
                let mut state = shared.state.lock();
                state.pending_intent = Some(PendingIntent {
                    descriptor: descriptor.clone(),
                    utterance,
                    response_language: response.language,
                });
                state.snapshot.metadata.phase = VoicePhase::NeedsReview;
                state.snapshot.metadata.route_kind = Some(RouteKind::Deterministic);
                let _ = shared.events.try_send(contract::PendingVoiceEvent {
                    owner: Some(context.owner.clone()),
                    operation_id: Some(context.operation_id),
                    event: VoiceEventKind::Intent { descriptor },
                });
                let _ = shared.events.try_send(contract::PendingVoiceEvent {
                    owner: Some(context.owner.clone()),
                    operation_id: Some(context.operation_id),
                    event: VoiceEventKind::StateChanged {
                        phase: VoicePhase::NeedsReview,
                        route_kind: Some(RouteKind::Deterministic),
                        cancel_available: false,
                        cleanup: state.snapshot.metadata.cleanup,
                    },
                });
                shared.changed.notify_all();
                return;
            }
            let answer_context = match VoiceSessionSupervisor::next_context(
                &shared,
                &context,
                GENERATION_DEADLINE,
            ) {
                Ok(context) => context,
                Err(error) => {
                    finish_operation(
                        &shared,
                        &context,
                        CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                        VoicePhase::Error,
                        Some(error),
                    );
                    return;
                }
            };
            if set_phase(
                &shared,
                &answer_context,
                VoicePhase::Routing,
                Some(response.kind),
            )
            .is_err()
            {
                return;
            }
            publish_answer_and_finish(AnswerPublication {
                shared,
                speech,
                context: answer_context,
                utterance,
                answer: response.text,
                answer_language: response.language,
                route_kind: response.kind,
                record_history,
            });
        }
        RouterDecision::Unhandled {
            utterance: model_utterance,
            response_language,
        } => {
            let Some(assets) = assets else {
                unavailable_answer(
                    shared,
                    speech,
                    context,
                    utterance,
                    response_language.unwrap_or(model_utterance.language),
                    record_history,
                );
                return;
            };
            if caller.ensure_live(&window).is_err() {
                context.cancel.cancel();
                finish_operation(
                    &shared,
                    &context,
                    CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                    VoicePhase::Error,
                    Some(VoiceError::Unauthorized),
                );
                return;
            }
            if readiness_from_asset(Some(assets.as_ref()), "qwen3-4b-q4-k-m").state
                != ReadinessState::Ready
            {
                unavailable_answer(
                    shared,
                    speech,
                    context,
                    utterance,
                    response_language.unwrap_or(model_utterance.language),
                    record_history,
                );
                return;
            }
            let needs = crate::voice_authorization::WorkerAuthorizationNeeds {
                asr: utterance.source == UtteranceSource::Speech,
                llm: true,
            };
            let worker_session = match cached_worker_authorization_session(
                &shared.worker_session,
                shared.worker_authority.as_ref(),
                &context.cancel,
                context.deadline + GENERATION_DEADLINE,
                needs,
            ) {
                Ok(session) => session,
                Err(error) => {
                    finish_operation(
                        &shared,
                        &context,
                        CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                        VoicePhase::Error,
                        Some(map_worker_authorization_error(error)),
                    );
                    return;
                }
            };
            let model = match assets.open_model("qwen3-4b-q4-k-m", AssetPurpose::LlmWeights) {
                Ok(model) => model,
                Err(_) => {
                    unavailable_answer(
                        shared,
                        speech,
                        context,
                        utterance,
                        response_language.unwrap_or(model_utterance.language),
                        record_history,
                    );
                    return;
                }
            };
            if caller.ensure_live(&window).is_err() {
                context.cancel.cancel();
                finish_operation(
                    &shared,
                    &context,
                    CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                    VoicePhase::Error,
                    Some(VoiceError::Unauthorized),
                );
                return;
            }
            let closure = match assets.runtime_closure(model.asset_id()) {
                Ok(closure) if closure.role() == crate::voice_authorization::WorkerRole::Llm => {
                    closure
                }
                _ => {
                    unavailable_answer(
                        shared,
                        speech,
                        context,
                        utterance,
                        response_language.unwrap_or(model_utterance.language),
                        record_history,
                    );
                    return;
                }
            };
            let llm_context = match VoiceSessionSupervisor::next_context(
                &shared,
                &context,
                GENERATION_DEADLINE,
            ) {
                Ok(context) => context,
                Err(error) => {
                    finish_operation(
                        &shared,
                        &context,
                        CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                        VoicePhase::Error,
                        Some(error),
                    );
                    return;
                }
            };
            if set_phase(
                &shared,
                &llm_context,
                VoicePhase::Thinking,
                Some(RouteKind::LocalModel),
            )
            .is_err()
            {
                return;
            }
            let history = {
                let state = shared.state.lock();
                clone_history_store(&state.history)
            };
            let mut backend = crate::voice::llama_backend::LlamaBackend::new(closure);
            if backend
                .generate(
                    llm_context.clone(),
                    model_utterance.clone(),
                    response_language.unwrap_or(model_utterance.language),
                    &history,
                    model,
                    Arc::clone(&worker_session),
                )
                .is_err()
            {
                unavailable_answer(
                    shared,
                    speech,
                    llm_context,
                    utterance,
                    response_language.unwrap_or(model_utterance.language),
                    record_history,
                );
                return;
            }
            let mut answer = String::new();
            loop {
                if caller.ensure_live(&window).is_err() {
                    llm_context.cancel.cancel();
                    let receipt = backend
                        .cancel(&llm_context)
                        .unwrap_or_else(|_| CleanupReceipt::failed());
                    finish_operation(
                        &shared,
                        &llm_context,
                        receipt,
                        VoicePhase::Error,
                        Some(VoiceError::Unauthorized),
                    );
                    return;
                }
                match backend.poll(MAX_TEXT_DELTA_BYTES) {
                    Ok(Some(contract::ConversationEvent::TextDelta {
                        owner,
                        operation_id,
                        sequence: _,
                        text,
                    })) if owner == llm_context.owner
                        && operation_id == llm_context.operation_id =>
                    {
                        if answer.len().saturating_add(text.len()) > MAX_MODEL_OUTPUT_BYTES {
                            llm_context.cancel.cancel();
                            let receipt = backend
                                .cancel(&llm_context)
                                .unwrap_or_else(|_| CleanupReceipt::failed());
                            finish_operation(
                                &shared,
                                &llm_context,
                                receipt,
                                VoicePhase::Error,
                                Some(VoiceError::TooLarge),
                            );
                            return;
                        }
                        answer.push_str(&text);
                        if publish_answer_delta(
                            &shared,
                            &llm_context,
                            response_language.unwrap_or(model_utterance.language),
                            &text,
                            false,
                            false,
                        )
                        .is_err()
                        {
                            llm_context.cancel.cancel();
                            let receipt = backend
                                .cancel(&llm_context)
                                .unwrap_or_else(|_| CleanupReceipt::failed());
                            finish_operation(
                                &shared,
                                &llm_context,
                                receipt,
                                VoicePhase::Error,
                                Some(VoiceError::Unavailable),
                            );
                            return;
                        }
                    }
                    Ok(Some(contract::ConversationEvent::Completed {
                        owner,
                        operation_id,
                        ..
                    })) if owner == llm_context.owner
                        && operation_id == llm_context.operation_id =>
                    {
                        break;
                    }
                    Ok(Some(contract::ConversationEvent::Failed {
                        owner,
                        operation_id,
                        ..
                    })) if owner == llm_context.owner
                        && operation_id == llm_context.operation_id =>
                    {
                        finish_operation(
                            &shared,
                            &llm_context,
                            CleanupReceipt::verified(CleanupCode::WorkerExited),
                            VoicePhase::Error,
                            Some(VoiceError::ProviderFailed),
                        );
                        return;
                    }
                    Ok(Some(_)) => {
                        llm_context.cancel.cancel();
                        let receipt = backend
                            .cancel(&llm_context)
                            .unwrap_or_else(|_| CleanupReceipt::failed());
                        finish_operation(
                            &shared,
                            &llm_context,
                            receipt,
                            VoicePhase::Error,
                            Some(VoiceError::ProtocolError),
                        );
                        return;
                    }
                    Ok(None) => {
                        if llm_context.is_cancelled_or_expired() {
                            let receipt = backend
                                .cancel(&llm_context)
                                .unwrap_or_else(|_| CleanupReceipt::failed());
                            finish_operation(
                                &shared,
                                &llm_context,
                                receipt,
                                VoicePhase::Error,
                                Some(VoiceError::Timeout),
                            );
                            return;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => {
                        finish_operation(
                            &shared,
                            &llm_context,
                            CleanupReceipt::verified(CleanupCode::WorkerExited),
                            VoicePhase::Error,
                            Some(VoiceError::ProviderFailed),
                        );
                        return;
                    }
                }
            }
            if answer.is_empty() {
                finish_operation(
                    &shared,
                    &llm_context,
                    CleanupReceipt::verified(CleanupCode::WorkerExited),
                    VoicePhase::Error,
                    Some(VoiceError::ProviderFailed),
                );
                return;
            }
            publish_answer_and_finish(AnswerPublication {
                shared,
                speech,
                context: llm_context,
                utterance,
                answer,
                answer_language: response_language.unwrap_or(model_utterance.language),
                route_kind: RouteKind::LocalModel,
                record_history,
            });
        }
    }
}

fn descriptor_owner(descriptor: &AppIntentDescriptor) -> &VoiceOwner {
    match descriptor {
        AppIntentDescriptor::Overview { owner, .. }
        | AppIntentDescriptor::Conversation { owner, .. }
        | AppIntentDescriptor::Motion { owner, .. } => owner,
    }
}

fn clone_history_store(
    history: &crate::voice::context::CompletedTurnStore,
) -> crate::voice::context::CompletedTurnStore {
    let mut clone = crate::voice::context::CompletedTurnStore::default();
    for turn in history.completed() {
        let _ = clone.commit(crate::voice::context::CompletedTurn::new(
            turn.turn_id.clone(),
            turn.user_language,
            turn.assistant_language,
            turn.user_text.clone(),
            turn.assistant_text.clone(),
        ));
    }
    clone
}

fn unavailable_answer(
    shared: Arc<VoiceShared>,
    speech: Arc<Mutex<Option<crate::voice::speech::SystemSpeechProvider>>>,
    context: TurnContext,
    utterance: AcceptedUtterance,
    language: Language,
    record_history: bool,
) {
    let text = match language {
        Language::En => {
            "Open-ended local conversation is unavailable until the approved local model is ready."
        }
        Language::Tr => "Onaylı yerel model hazır olana kadar serbest yerel sohbet kullanılamıyor.",
    }
    .to_owned();
    {
        let mut state = shared.state.lock();
        state
            .snapshot
            .metadata
            .notices
            .push(NoticeCode::ModelUnavailable);
    }
    publish_answer_and_finish(AnswerPublication {
        shared,
        speech,
        context,
        utterance,
        answer: text,
        answer_language: language,
        route_kind: RouteKind::Unavailable,
        record_history,
    });
}

struct AnswerPublication {
    shared: Arc<VoiceShared>,
    speech: Arc<Mutex<Option<crate::voice::speech::SystemSpeechProvider>>>,
    context: TurnContext,
    utterance: AcceptedUtterance,
    answer: String,
    answer_language: Language,
    route_kind: RouteKind,
    record_history: bool,
}

fn publish_answer_and_finish(publication: AnswerPublication) {
    let AnswerPublication {
        shared,
        speech,
        context,
        utterance,
        answer,
        answer_language,
        route_kind,
        record_history,
    } = publication;
    if answer.is_empty() {
        finish_operation(
            &shared,
            &context,
            CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
            VoicePhase::Error,
            Some(VoiceError::ProviderFailed),
        );
        return;
    }
    let chunks = split_utf8(&answer);
    if chunks.len() > MAX_EVENT_BATCH_RECORDS * 4 {
        finish_operation(
            &shared,
            &context,
            CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
            VoicePhase::Error,
            Some(VoiceError::TooLarge),
        );
        return;
    }
    for (index, chunk) in chunks.iter().enumerate() {
        if publish_answer_delta(
            &shared,
            &context,
            answer_language,
            chunk,
            index + 1 == chunks.len(),
            false,
        )
        .is_err()
        {
            finish_operation(
                &shared,
                &context,
                CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                VoicePhase::Error,
                Some(VoiceError::Unavailable),
            );
            return;
        }
    }
    {
        let mut state = shared.state.lock();
        if record_history
            && commit_completed_turn(&mut state, &context, &utterance, &answer, answer_language)
                .is_err()
        {
            drop(state);
            finish_operation(
                &shared,
                &context,
                CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                VoicePhase::Error,
                Some(VoiceError::TooLarge),
            );
            return;
        } else if !record_history {
            state.snapshot.pending.clear();
        }
        state.snapshot.metadata.route_kind = Some(route_kind);
    }
    let (spoken_reply, voice_id) = {
        let state = shared.state.lock();
        let voice_id = match answer_language {
            Language::Tr => state.snapshot.metadata.preferences.voice_ids.tr.clone(),
            Language::En => state.snapshot.metadata.preferences.voice_ids.en.clone(),
        };
        (state.snapshot.metadata.preferences.spoken_reply, voice_id)
    };
    if !spoken_reply || answer.is_empty() {
        {
            let mut state = shared.state.lock();
            state.snapshot.metadata.notices.push(NoticeCode::NotSpoken);
        }
        finish_operation(
            &shared,
            &context,
            CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
            VoicePhase::Idle,
            None,
        );
        return;
    }
    let Some(voice_id) = voice_id else {
        {
            let mut state = shared.state.lock();
            state
                .snapshot
                .metadata
                .notices
                .push(NoticeCode::SpeechUnavailable);
        }
        finish_operation(
            &shared,
            &context,
            CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
            VoicePhase::Idle,
            None,
        );
        return;
    };
    if contract::validate_spoken_request(&answer).is_err() {
        finish_operation(
            &shared,
            &context,
            CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
            VoicePhase::Error,
            Some(VoiceError::TooLarge),
        );
        return;
    }
    if ensure_speech_provider(&speech).is_err() {
        let mut state = shared.state.lock();
        state
            .snapshot
            .metadata
            .notices
            .push(NoticeCode::SpeechUnavailable);
        drop(state);
        finish_operation(
            &shared,
            &context,
            CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
            VoicePhase::Idle,
            None,
        );
        return;
    }
    let mut pieces = split_spoken_text(&answer, 280, 1120);
    if pieces.len() > 3 {
        pieces.truncate(3);
        if let Some(last) = pieces.last_mut() {
            while !last.is_empty() && !last.ends_with(['.', '?', '!']) {
                last.pop();
            }
        }
        let mut state = shared.state.lock();
        state.snapshot.metadata.notices.push(NoticeCode::NotSpoken);
    }
    {
        let mut state = shared.state.lock();
        state.snapshot.metadata.phase = VoicePhase::Speaking;
        state.snapshot.metadata.output_progress = OutputProgress {
            queued_segments: pieces.len() as u8,
            completed_segments: 0,
            active: false,
        };
    }
    for (index, text) in pieces.into_iter().enumerate() {
        if context.is_cancelled_or_expired() {
            cancel_speech(&speech, &context);
            finish_operation(
                &shared,
                &context,
                CleanupReceipt::failed(),
                VoicePhase::Error,
                Some(VoiceError::Cancelled),
            );
            return;
        }
        let segment_context = if index == 0 {
            context.clone()
        } else {
            match VoiceSessionSupervisor::next_context(&shared, &context, SPEECH_DEADLINE) {
                Ok(context) => context,
                Err(error) => {
                    finish_operation(
                        &shared,
                        &context,
                        CleanupReceipt::failed(),
                        VoicePhase::Error,
                        Some(error),
                    );
                    return;
                }
            }
        };
        let segment = contract::SpeechSegment {
            text,
            language: answer_language,
            voice_id: contract::VoiceId(voice_id.clone()),
        };
        let accepted = speech
            .lock()
            .as_mut()
            .ok_or(contract::SpeechError::Unavailable)
            .and_then(|provider| provider.speak(&segment_context, segment));
        if accepted.is_err() {
            {
                let mut state = shared.state.lock();
                state.snapshot.metadata.notices.push(NoticeCode::NotSpoken);
            }
            let _ = cancel_speech(&speech, &segment_context);
            finish_operation(
                &shared,
                &segment_context,
                CleanupReceipt::verified(CleanupCode::SpeechStopped),
                VoicePhase::Idle,
                None,
            );
            return;
        }
    }
}

fn split_spoken_text(text: &str, max_scalars: usize, max_bytes: usize) -> Vec<String> {
    let mut output = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if candidate.chars().count() > max_scalars || candidate.len() > max_bytes {
            if !current.is_empty() {
                output.push(std::mem::take(&mut current));
            }
            if word.chars().count() > max_scalars || word.len() > max_bytes {
                let mut piece = String::new();
                for character in word.chars() {
                    let mut next = piece.clone();
                    next.push(character);
                    if (next.chars().count() > max_scalars || next.len() > max_bytes)
                        && !piece.is_empty()
                    {
                        output.push(std::mem::take(&mut piece));
                    }
                    piece.push(character);
                }
                if !piece.is_empty() {
                    current = piece;
                }
            } else {
                current.push_str(word);
            }
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        output.push(current);
    }
    output
}

fn ensure_speech_provider(
    speech: &Arc<Mutex<Option<crate::voice::speech::SystemSpeechProvider>>>,
) -> Result<(), contract::SpeechError> {
    let mut provider = speech.lock();
    if provider.is_none() {
        *provider = Some(crate::voice::speech::SystemSpeechProvider::system()?);
    }
    Ok(())
}
fn cancel_speech(
    speech: &Mutex<Option<crate::voice::speech::SystemSpeechProvider>>,
    context: &TurnContext,
) -> CleanupReceipt {
    speech.lock().as_mut().map_or_else(
        || CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
        |provider| {
            contract::LocalSpeechProvider::cancel(provider, context)
                .unwrap_or_else(|_| CleanupReceipt::failed())
        },
    )
}

impl VoiceSessionSupervisor {
    pub fn cancel(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        window: &tauri::WebviewWindow,
        request: CancelRequest,
    ) -> Result<VoiceCommandWireResult, VoiceError> {
        self.authorize(&caller, &VoiceCommand::Cancel(request.clone()))?;
        caller.ensure_live(window)?;
        let context = {
            let mut state = self.shared.state.lock();
            if state.snapshot.owner.as_ref() != Some(&request.owner) {
                return Err(VoiceError::StaleOwner);
            }
            let context = state.active.as_ref().map(|active| active.context.clone());
            if let Some(context) = context {
                context.cancel.cancel();
                state.snapshot.metadata.phase = VoicePhase::Cancelling;
                state.snapshot.metadata.cancel_available = false;
                state.snapshot.metadata.error = None;
                let _ = self.shared.events.try_send(contract::PendingVoiceEvent {
                    owner: Some(request.owner.clone()),
                    operation_id: Some(context.operation_id),
                    event: VoiceEventKind::StateChanged {
                        phase: VoicePhase::Cancelling,
                        route_kind: state.snapshot.metadata.route_kind,
                        cancel_available: false,
                        cleanup: state.snapshot.metadata.cleanup,
                    },
                });
                self.shared.changed.notify_all();
                Some(context)
            } else {
                None
            }
        };
        let Some(context) = context else {
            return self.command_result(caller, false, VoiceResultCode::NoChange);
        };
        let receipt = self.cancel_providers(&context);
        if receipt.verified {
            finish_operation(&self.shared, &context, receipt, VoicePhase::Idle, None);
            Ok(VoiceCommandWireResult {
                accepted: true,
                code: VoiceResultCode::Completed,
                transfer: Some(self.snapshot_header(caller)?),
            })
        } else {
            let mut state = self.shared.state.lock();
            state.locked = true;
            state.snapshot.metadata.phase = VoicePhase::Error;
            state.snapshot.metadata.cleanup = receipt;
            state.snapshot.metadata.error = Some(VoiceError::CleanupFailed);
            Err(VoiceError::CleanupFailed)
        }
    }

    fn cancel_providers(&self, context: &TurnContext) -> CleanupReceipt {
        context.cancel.cancel();
        invalidate_worker_authorization_session(&self.shared);
        let mut receipt = CleanupReceipt::verified(CleanupCode::NoResourcesOwned);
        let capture_receipt = self.capture.lock().as_mut().map_or_else(
            || CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
            |provider| provider.cancel(&context.owner),
        );
        if !capture_receipt.verified || capture_receipt.code != CleanupCode::NoResourcesOwned {
            receipt = capture_receipt;
        }
        let speech_receipt = self.speech.lock().as_mut().map_or_else(
            || CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
            |provider| {
                contract::LocalSpeechProvider::cancel(provider, context)
                    .unwrap_or_else(|_| CleanupReceipt::failed())
            },
        );
        if !speech_receipt.verified || speech_receipt.code != CleanupCode::NoResourcesOwned {
            receipt = speech_receipt;
        }
        let deadline = Instant::now() + WORKER_REAP_DEADLINE + Duration::from_millis(100);
        let mut state = self.shared.state.lock();
        while state
            .active
            .as_ref()
            .is_some_and(|active| active.context.owner == context.owner)
            && receipt.verified
            && Instant::now() < deadline
        {
            self.shared
                .changed
                .wait_for(&mut state, Duration::from_millis(20));
        }
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.context.owner == context.owner)
        {
            receipt = CleanupReceipt::failed();
        }
        if !receipt.verified {
            state.locked = true;
            state.snapshot.metadata.cleanup = receipt;
            state.snapshot.metadata.error = Some(VoiceError::CleanupFailed);
        }
        receipt
    }

    pub fn clear_session(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        window: &tauri::WebviewWindow,
    ) -> Result<VoiceCommandWireResult, VoiceError> {
        self.authorize(&caller, &VoiceCommand::ClearSession)?;
        caller.ensure_live(window)?;
        let active = self
            .shared
            .state
            .lock()
            .active
            .as_ref()
            .map(|active| active.context.clone());
        if let Some(context) = active {
            let receipt = self.cancel_providers(&context);
            if !receipt.verified {
                return Err(VoiceError::CleanupFailed);
            }
            finish_operation(&self.shared, &context, receipt, VoicePhase::Idle, None);
        }
        let mut state = self.shared.state.lock();
        if state.snapshot.metadata.phase == VoicePhase::Speaking
            || state.snapshot.metadata.phase == VoicePhase::Cancelling
            || state.locked
        {
            return Err(VoiceError::CleanupFailed);
        }
        state.history.clear();
        state.snapshot.history.clear();
        state.snapshot.pending.clear();
        state.pending_review = None;
        state.pending_intent = None;
        state.snapshot.owner = None;
        state.snapshot.metadata.phase = if state.snapshot.metadata.readiness.product_ready {
            VoicePhase::Idle
        } else {
            VoicePhase::SetupRequired
        };
        state.snapshot.metadata.cancel_available = false;
        state.snapshot.metadata.route_kind = None;
        state.snapshot.metadata.error = None;
        state.snapshot.metadata.output_progress = OutputProgress {
            queued_segments: 0,
            completed_segments: 0,
            active: false,
        };
        drop(state);
        self.command_result(caller, true, VoiceResultCode::Completed)
    }

    pub fn select_preferences(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        window: &tauri::WebviewWindow,
        request: SelectPreferencesRequest,
    ) -> Result<VoiceCommandWireResult, VoiceError> {
        self.authorize(&caller, &VoiceCommand::SelectPreferences(request.clone()))?;
        caller.ensure_live(window)?;
        let mut state = self.shared.state.lock();
        if state.active.is_some()
            || state.pending_review.is_some()
            || state.pending_intent.is_some()
        {
            return Err(VoiceError::Busy);
        }
        let selected_device = request
            .input_device_handle
            .as_ref()
            .map(|handle| {
                state
                    .snapshot
                    .input_devices
                    .iter()
                    .find(|device| &device.handle.0 == handle)
                    .map(|device| device.handle.0.clone())
                    .ok_or(VoiceError::StaleCatalog)
            })
            .transpose()?;
        let validate_voice = |requested: &Option<String>, language: Language| {
            requested
                .as_ref()
                .map(|id| {
                    state
                        .snapshot
                        .voices
                        .iter()
                        .find(|voice| voice.id.0 == *id && voice.language == language)
                        .map(|voice| voice.id.0.clone())
                        .ok_or(VoiceError::StaleCatalog)
                })
                .transpose()
        };
        let tr_voice = validate_voice(&request.voice_ids.tr, Language::Tr)?;
        let en_voice = validate_voice(&request.voice_ids.en, Language::En)?;
        let fallback_device = state
            .snapshot
            .input_devices
            .iter()
            .find(|device| device.is_default)
            .map(|device| device.handle.0.clone());
        state.snapshot.metadata.preferences = VoicePreferences {
            language_mode: request.language_mode.or(state
                .snapshot
                .metadata
                .preferences
                .language_mode),
            input_device_handle: selected_device.or(fallback_device),
            voice_ids: VoiceLanguageIds {
                tr: tr_voice,
                en: en_voice,
            },
            spoken_reply: request.spoken_reply,
            review_before_send: request.review_before_send,
        };
        drop(state);
        self.command_result(caller, true, VoiceResultCode::Completed)
    }

    pub fn ack_intent(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        window: &tauri::WebviewWindow,
        request: AckIntentRequest,
    ) -> Result<VoiceCommandWireResult, VoiceError> {
        self.authorize(&caller, &VoiceCommand::AckIntent(request.clone()))?;
        caller.ensure_live(window)?;
        let pending = {
            let mut state = self.shared.state.lock();
            let pending = state.pending_intent.take().ok_or(VoiceError::StaleOwner)?;
            if descriptor_owner(&pending.descriptor) != &request.owner
                || descriptor_intent_id(&pending.descriptor) != request.intent_id.0
                || state.snapshot.owner.as_ref() != Some(&request.owner)
            {
                state.pending_intent = Some(pending);
                return Err(VoiceError::StaleOwner);
            }
            let Some(active) = state
                .active
                .as_ref()
                .filter(|active| active.context.owner == request.owner)
            else {
                state.pending_intent = Some(pending);
                return Err(VoiceError::StaleOwner);
            };
            let context = active.context.clone();
            if request.accepted {
                state.snapshot.metadata.phase = VoicePhase::Routing;
                Some((pending, context))
            } else {
                state.active = None;
                state.snapshot.pending.clear();
                state.snapshot.metadata.phase = VoicePhase::Idle;
                state.snapshot.metadata.cancel_available = false;
                state.snapshot.metadata.cleanup =
                    CleanupReceipt::verified(CleanupCode::NoResourcesOwned);
                let _ = self.shared.events.try_send(contract::PendingVoiceEvent {
                    owner: Some(request.owner.clone()),
                    operation_id: Some(context.operation_id),
                    event: VoiceEventKind::StateChanged {
                        phase: VoicePhase::Idle,
                        route_kind: Some(RouteKind::Deterministic),
                        cancel_available: false,
                        cleanup: state.snapshot.metadata.cleanup,
                    },
                });
                self.shared.changed.notify_all();
                None
            }
        };
        let Some((pending, context)) = pending else {
            return self.command_result(caller, true, VoiceResultCode::Completed);
        };
        let text = match pending.response_language {
            Language::En => "Confirmed. The interface will handle the requested local view.",
            Language::Tr => "Onaylandı. İstenen yerel görünümü arayüz yönetecek.",
        }
        .to_owned();
        publish_answer_and_finish(AnswerPublication {
            shared: Arc::clone(&self.shared),
            speech: Arc::clone(&self.speech),
            context,
            utterance: pending.utterance,
            answer: text,
            answer_language: pending.response_language,
            route_kind: RouteKind::Deterministic,
            record_history: true,
        });
        self.command_result(caller, true, VoiceResultCode::Completed)
    }

    pub fn test_output(
        &self,
        caller: crate::voice::ipc::NativeCallerContext,
        window: &tauri::WebviewWindow,
        request: TestOutputRequest,
    ) -> Result<VoiceCommandWireResult, VoiceError> {
        self.authorize(&caller, &VoiceCommand::TestOutput(request.clone()))?;
        caller.ensure_live(window)?;
        ensure_speech_provider(&self.speech).map_err(|_| VoiceError::Unavailable)?;
        let voice = {
            let mut speech = self.speech.lock();
            let provider = speech.as_mut().ok_or(VoiceError::Unavailable)?;
            provider
                .enumerate_voices()
                .map_err(|_| VoiceError::Unavailable)?
                .get(request.language)
                .into_iter()
                .flatten()
                .find(|voice| voice.id.0 == request.voice_id)
                .cloned()
                .ok_or(VoiceError::StaleCatalog)?
        };
        let context = self.begin_turn(
            LanguageMode::from(request.language),
            UtteranceSource::Typed,
            VoicePhase::Speaking,
            SPEECH_DEADLINE,
        )?;
        if caller.ensure_live(window).is_err() {
            context.cancel.cancel();
            let receipt = self.cancel_providers(&context);
            finish_operation(
                &self.shared,
                &context,
                receipt,
                VoicePhase::Error,
                Some(VoiceError::Unauthorized),
            );
            return Err(VoiceError::Unauthorized);
        }
        let text = match request.language {
            Language::En => "This is an explicitly requested local voice output test.",
            Language::Tr => "Bu, açıkça istenmiş yerel ses çıkışı testidir.",
        };
        self.speech
            .lock()
            .as_mut()
            .ok_or(VoiceError::Unavailable)?
            .speak(
                &context,
                contract::SpeechSegment {
                    text: text.to_owned(),
                    language: request.language,
                    voice_id: voice.id,
                },
            )
            .map_err(|_| VoiceError::Unavailable)?;
        self.command_result(caller, true, VoiceResultCode::Pending)
    }

    pub fn stop_for_window_loss(&self, reason: NativeStopReason) -> CleanupWaitHandle {
        invalidate_worker_authorization_session(&self.shared);
        let context = {
            let mut state = self.shared.state.lock();
            let context = state.active.as_ref().map(|active| active.context.clone());
            if let Some(context) = context {
                if reason == NativeStopReason::AssetRevoked {
                    context.cancel.cancel_for_authorization_change();
                } else {
                    context.cancel.cancel();
                }
                state.snapshot.metadata.phase = VoicePhase::Cancelling;
                state.snapshot.metadata.cancel_available = false;
                Some(context)
            } else {
                if matches!(
                    reason,
                    NativeStopReason::ApplicationLocked | NativeStopReason::ApplicationExit
                ) {
                    state.locked = true;
                    state.shutting_down = true;
                }
                None
            }
        };
        let shared = Arc::clone(&self.shared);
        let capture = Arc::clone(&self.capture);
        let speech = Arc::clone(&self.speech);
        if let Some(context) = context {
            let _ = thread::Builder::new()
                .name("saturday-voice-lifecycle-stop".to_owned())
                .spawn(move || {
                    let capture_receipt = capture.lock().as_mut().map_or_else(
                        || CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                        |provider| provider.cancel(&context.owner),
                    );
                    let speech_receipt = speech.lock().as_mut().map_or_else(
                        || CleanupReceipt::verified(CleanupCode::NoResourcesOwned),
                        |provider| {
                            contract::LocalSpeechProvider::cancel(provider, &context)
                                .unwrap_or_else(|_| CleanupReceipt::failed())
                        },
                    );
                    if !capture_receipt.verified || !speech_receipt.verified {
                        let mut state = shared.state.lock();
                        state.locked = true;
                        state.snapshot.metadata.cleanup = CleanupReceipt::failed();
                        state.snapshot.metadata.error = Some(VoiceError::CleanupFailed);
                        state.snapshot.metadata.phase = VoicePhase::Error;
                        shared.changed.notify_all();
                        return;
                    }
                    let deadline = Instant::now() + WORKER_REAP_DEADLINE;
                    let mut state = shared.state.lock();
                    while state
                        .active
                        .as_ref()
                        .is_some_and(|active| active.context.owner == context.owner)
                        && Instant::now() < deadline
                    {
                        shared
                            .changed
                            .wait_for(&mut state, Duration::from_millis(20));
                    }
                    if state
                        .active
                        .as_ref()
                        .is_some_and(|active| active.context.owner == context.owner)
                    {
                        state.locked = true;
                        state.snapshot.metadata.cleanup = CleanupReceipt::failed();
                        state.snapshot.metadata.error = Some(VoiceError::CleanupFailed);
                        state.snapshot.metadata.phase = VoicePhase::Error;
                    } else {
                        state.snapshot.metadata.cleanup =
                            if capture_receipt.code != CleanupCode::NoResourcesOwned {
                                capture_receipt
                            } else if speech_receipt.code != CleanupCode::NoResourcesOwned {
                                speech_receipt
                            } else {
                                CleanupReceipt::verified(CleanupCode::NoResourcesOwned)
                            };
                    }
                    if matches!(
                        reason,
                        NativeStopReason::ApplicationLocked | NativeStopReason::ApplicationExit
                    ) {
                        state.locked = true;
                        state.shutting_down = true;
                    }
                    shared.changed.notify_all();
                });
        } else {
            let mut state = self.shared.state.lock();
            if matches!(
                reason,
                NativeStopReason::ApplicationLocked | NativeStopReason::ApplicationExit
            ) {
                state.locked = true;
                state.shutting_down = true;
            }
            self.shared.changed.notify_all();
        }
        self.cleanup_wait_handle()
    }

    pub fn stop_for_app_exit(&self) -> CleanupWaitHandle {
        self.stop_for_window_loss(NativeStopReason::ApplicationExit)
    }
}

impl CleanupWaitHandle {
    pub fn wait_for_cleanup(&self, timeout: Duration) -> CleanupReceipt {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.state.lock();
        while state.active.is_some() && Instant::now() < deadline {
            self.shared.changed.wait_for(
                &mut state,
                deadline.saturating_duration_since(Instant::now()),
            );
        }
        state.snapshot.metadata.cleanup
    }
}

impl Drop for VoiceSessionSupervisor {
    fn drop(&mut self) {
        self.shared.monitor_shutdown.store(true, Ordering::Release);
        let _ = self.stop_for_app_exit();
        if let Some(monitor) = self.event_monitor.lock().take() {
            let _ = monitor.join();
        }
    }
}

fn descriptor_intent_id(descriptor: &AppIntentDescriptor) -> u64 {
    match descriptor {
        AppIntentDescriptor::Overview { intent_id, .. }
        | AppIntentDescriptor::Conversation { intent_id, .. }
        | AppIntentDescriptor::Motion { intent_id, .. } => intent_id.0,
    }
}

impl VoiceSessionSupervisor {
    pub fn asset_manager(&self) -> Option<Arc<dyn AssetManager>> {
        self.asset_manager.as_ref().map(Arc::clone)
    }
}
