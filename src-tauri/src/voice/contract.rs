pub use crate::voice::context::{
    validate_spoken_request, validate_typed_request, RequestLimitError,
};
pub use saturday_voice_types::{
    AsrError, AsrLanguage, AsrQualityFlag, AsrResult, CancelToken, CancellationReason,
    ConversationFailure, Language, LanguageEvidence, LanguageMode, TranscriptEligibility,
};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;

pub const PUBLIC_PROTOCOL_VERSION: u8 = 1;
pub const PRIVATE_PROTOCOL_VERSION: u8 = 2;
pub const MAX_PUBLIC_FRAME_BYTES: usize = 65_536;
pub const MAX_JSON_DEPTH: usize = 32;
pub const MAX_JSON_NODES: usize = 8_192;
pub const MAX_JSON_STRING_BYTES: usize = 8_192;
pub const MAX_TEXT_DELTA_BYTES: usize = 4_096;
pub const MAX_TYPED_SCALARS: usize = 500;
pub const MAX_TYPED_BYTES: usize = 2_000;
pub const MAX_SPOKEN_SCALARS: usize = 2_000;
pub const MAX_SPOKEN_BYTES: usize = 8_000;
pub const MAX_HISTORY_MESSAGES: usize = 40;
pub const MAX_HISTORY_BYTES: usize = 65_536;
pub const MAX_CAPTURE_SECONDS: u32 = 30;
pub const MAX_CAPTURE_SAMPLES: u32 = 480_000;
pub const CANONICAL_SAMPLE_RATE_HZ: u32 = 16_000;
pub const MAX_RAW_AUDIO_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EVENT_QUEUE_RECORDS: usize = 64;
pub const MAX_EVENT_BATCH_RECORDS: usize = 16;
pub const MAX_SNAPSHOT_PAGES: usize = 128;
pub const MAX_SNAPSHOT_TEXT_BYTES: usize = 81_728;
pub const MAX_SNAPSHOT_METADATA_BYTES: usize = 256 * 1024;
pub const MAX_SNAPSHOT_PAGE_TEXT_BYTES: usize = 4_096;
pub const MAX_SNAPSHOT_PAGE_METADATA_BYTES: usize = 48 * 1024;
pub const MAX_DEVICE_CATALOG_RECORDS: usize = 64;
pub const MAX_SUPPORTED_VOICES: usize = 128;
pub const MAX_CONTROL_FRAME_BYTES: usize = 65_536;
pub const MAX_MODEL_CONTEXT_TOKENS: usize = 4_096;
pub const MAX_MODEL_OUTPUT_TOKENS: usize = 256;
pub const MAX_MODEL_OUTPUT_BYTES: usize = 8_192;
pub const ASR_DEADLINE: Duration = Duration::from_secs(15);
pub const GENERATION_DEADLINE: Duration = Duration::from_secs(60);
pub const SPEECH_DEADLINE: Duration = Duration::from_secs(90);
pub const CANCEL_ACK_DEADLINE: Duration = Duration::from_millis(250);
pub const WORKER_REAP_DEADLINE: Duration = Duration::from_secs(2);
pub const SPEECH_TAIL_GUARD: Duration = Duration::from_millis(250);
pub const MAX_OWNER_ID_BYTES: usize = 128;

fn deserialize_owner_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty() || value.len() > MAX_OWNER_ID_BYTES || value.chars().any(char::is_control) {
        return Err(serde::de::Error::custom("invalid owner identifier"));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecimalU64(pub u64);

impl Serialize for DecimalU64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for DecimalU64 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DecimalVisitor;

        impl<'de> Visitor<'de> for DecimalVisitor {
            type Value = DecimalU64;

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
                    return Err(E::custom("noncanonical decimal u64"));
                }
                value
                    .parse::<u64>()
                    .map(DecimalU64)
                    .map_err(|_| E::custom("decimal u64 overflow"))
            }
        }

        deserializer.deserialize_str(DecimalVisitor)
    }
}

mod decimal_u64 {
    use super::DecimalU64;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        DecimalU64(*value).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        DecimalU64::deserialize(deserializer).map(|value| value.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(pub u64);

impl Serialize for OperationId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        DecimalU64(self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for OperationId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        DecimalU64::deserialize(deserializer).map(|value| Self(value.0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceOwner {
    #[serde(deserialize_with = "deserialize_owner_id")]
    pub session_id: String,
    #[serde(deserialize_with = "deserialize_owner_id")]
    pub turn_id: String,
    #[serde(with = "decimal_u64")]
    pub generation: u64,
}

pub struct TurnContext {
    pub owner: VoiceOwner,
    pub operation_id: OperationId,
    pub cancel: CancelToken,
    pub deadline: Instant,
    pub event_sender: Arc<dyn VoiceEventSender>,
}

impl fmt::Debug for TurnContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TurnContext")
            .field("owner", &self.owner)
            .field("operation_id", &self.operation_id)
            .field("cancelled", &self.cancel.is_cancelled())
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

impl Clone for TurnContext {
    fn clone(&self) -> Self {
        Self {
            owner: self.owner.clone(),
            operation_id: self.operation_id,
            cancel: self.cancel.clone(),
            deadline: self.deadline,
            event_sender: self.event_sender.clone(),
        }
    }
}

impl TurnContext {
    pub fn is_cancelled_or_expired(&self) -> bool {
        self.cancel.is_cancelled() || Instant::now() >= self.deadline
    }

    pub fn try_send_event(&self, event: VoiceEventKind) -> Result<(), EventQueueError> {
        self.event_sender.try_send(PendingVoiceEvent {
            owner: Some(self.owner.clone()),
            operation_id: Some(self.operation_id),
            event,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoicePhase {
    Unavailable,
    SetupRequired,
    Idle,
    RequestingPermission,
    Listening,
    Transcribing,
    NeedsReview,
    Routing,
    Thinking,
    Speaking,
    Cancelling,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessState {
    Unavailable,
    SetupRequired,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderReadiness {
    pub state: ReadinessState,
    pub code: ReadinessCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessCode {
    NotInspected,
    SourceReviewRequired,
    RuntimeBuildRequired,
    OwnerTrustSetupRequired,
    AssetApprovalRequired,
    AssetUnavailable,
    ProviderUnavailable,
    PermissionRequired,
    OfflineEvidenceRequired,
    QualityEvidenceRequired,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineTaskCounters {
    #[serde(with = "decimal_u64")]
    pub completed: u64,
    #[serde(with = "decimal_u64")]
    pub without_llm: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessSnapshot {
    pub capture: ProviderReadiness,
    pub asr: ProviderReadiness,
    pub router: ProviderReadiness,
    pub llm: ProviderReadiness,
    pub speech: ProviderReadiness,
    pub routine_tasks: RoutineTaskCounters,
    pub product_ready: bool,
}

impl ReadinessSnapshot {
    pub fn setup_blocked() -> Self {
        let blocked = ProviderReadiness {
            state: ReadinessState::SetupRequired,
            code: ReadinessCode::AssetApprovalRequired,
        };
        Self {
            capture: blocked.clone(),
            asr: blocked.clone(),
            router: ProviderReadiness {
                state: ReadinessState::Ready,
                code: ReadinessCode::Ready,
            },
            llm: blocked.clone(),
            speech: blocked,
            routine_tasks: RoutineTaskCounters {
                completed: 0,
                without_llm: 0,
            },
            product_ready: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceSnapshotMetadata {
    pub session_id: String,
    pub phase: VoicePhase,
    pub preferences: VoicePreferences,
    pub resolved_language: Option<Language>,
    pub readiness: ReadinessSnapshot,
    pub voice_catalog: VoiceCatalogRevision,
    pub cancel_available: bool,
    pub cleanup: CleanupReceipt,
    pub route_kind: Option<RouteKind>,
    pub capture_elapsed_ms: Option<u32>,
    pub output_progress: OutputProgress,
    #[serde(with = "decimal_u64")]
    pub next_transcript_sequence: u64,
    #[serde(with = "decimal_u64")]
    pub next_answer_sequence: u64,
    pub notices: Vec<NoticeCode>,
    pub error: Option<VoiceError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoicePreferences {
    pub language_mode: Option<LanguageMode>,
    pub input_device_handle: Option<String>,
    pub voice_ids: VoiceLanguageIds,
    pub spoken_reply: bool,
    pub review_before_send: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceLanguageIds {
    pub tr: Option<String>,
    pub en: Option<String>,
}

impl Default for VoicePreferences {
    fn default() -> Self {
        Self {
            language_mode: None,
            input_device_handle: None,
            voice_ids: VoiceLanguageIds { tr: None, en: None },
            spoken_reply: false,
            review_before_send: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupCode {
    NoResourcesOwned,
    CaptureStopped,
    WorkerExited,
    SpeechStopped,
    Verified,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupReceipt {
    pub stopped: bool,
    pub verified: bool,
    pub code: CleanupCode,
}

impl CleanupReceipt {
    pub const fn verified(code: CleanupCode) -> Self {
        Self {
            stopped: true,
            verified: true,
            code,
        }
    }

    pub const fn failed() -> Self {
        Self {
            stopped: false,
            verified: false,
            code: CleanupCode::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "snake_case")]
pub enum VoiceError {
    #[error("invalid_request")]
    InvalidRequest,
    #[error("unauthorized")]
    Unauthorized,
    #[error("unavailable")]
    Unavailable,
    #[error("setup_required")]
    SetupRequired,
    #[error("busy")]
    Busy,
    #[error("stale_owner")]
    StaleOwner,
    #[error("stale_catalog")]
    StaleCatalog,
    #[error("unsupported")]
    Unsupported,
    #[error("permission_denied")]
    PermissionDenied,
    #[error("too_large")]
    TooLarge,
    #[error("timeout")]
    Timeout,
    #[error("cancelled")]
    Cancelled,
    #[error("provider_failed")]
    ProviderFailed,
    #[error("protocol_error")]
    ProtocolError,
    #[error("sequence_exhausted")]
    SequenceExhausted,
    #[error("transfer_expired")]
    TransferExpired,
    #[error("cleanup_failed")]
    CleanupFailed,
    #[error("internal")]
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicCommand {
    GetStatus,
    Start,
    Finish,
    SubmitText,
    ConfirmTranscript,
    Cancel,
    ClearSession,
    SelectPreferences,
    AckIntent,
    AssetsGetStatus,
    AssetsRequestInstall,
    AssetsDisable,
    AssetsDelete,
    AssetsRollback,
    TestOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceCommand {
    GetStatus(StatusRequest),
    Start(StartRequest),
    Finish(OwnerRequest),
    SubmitText(SubmitTextRequest),
    ConfirmTranscript(ConfirmTranscriptRequest),
    Cancel(CancelRequest),
    ClearSession,
    SelectPreferences(SelectPreferencesRequest),
    AckIntent(AckIntentRequest),
    AssetsGetStatus,
    AssetsRequestInstall(AssetInstallRequest),
    AssetsDisable(AssetRequest),
    AssetsDelete(AssetRequest),
    AssetsRollback(AssetRollbackRequest),
    TestOutput(TestOutputRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum StatusRequest {
    SnapshotBegin {},
    SnapshotNext {
        resync_id: DecimalU64,
        cursor: DecimalU64,
    },
    Events {
        after_event_sequence: DecimalU64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub language_mode: LanguageMode,
    #[serde(deserialize_with = "required_nullable")]
    pub input_device_handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerRequest {
    pub owner: VoiceOwner,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitTextRequest {
    pub text: String,
    pub language_mode: LanguageMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmTranscriptRequest {
    pub owner: VoiceOwner,
    pub text: String,
    pub language: Language,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    User,
    SurfaceClosed,
    WindowHidden,
    WindowUnfocused,
    ModuleDisabled,
    Shutdown,
    Timeout,
    ProviderFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelRequest {
    pub owner: VoiceOwner,
    pub reason: CancelReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectPreferencesRequest {
    pub language_mode: Option<LanguageMode>,
    #[serde(deserialize_with = "required_nullable")]
    pub input_device_handle: Option<String>,
    pub voice_ids: VoiceLanguageIds,
    pub spoken_reply: bool,
    pub review_before_send: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AckIntentRequest {
    pub owner: VoiceOwner,
    pub intent_id: DecimalU64,
    pub accepted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetPurpose {
    Runtime,
    AsrWeights,
    LlmWeights,
    PortableVoice,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetInstallRequest {
    pub asset_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRequest {
    pub asset_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRollbackRequest {
    pub asset_id: String,
    pub version_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestOutputRequest {
    pub language: Language,
    pub voice_id: String,
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Debug)]
enum StrictValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<StrictValue>),
    Object(BTreeMap<String, StrictValue>),
}

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StrictValueVisitor;

        impl<'de> Visitor<'de> for StrictValueVisitor {
            type Value = StrictValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded JSON value without duplicate object keys")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue::Null)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue::Null)
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue::Bool(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue::Number(value.into()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue::Number(value.into()))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Number::from_f64(value)
                    .map(StrictValue::Number)
                    .ok_or_else(|| E::custom("invalid JSON number"))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue::String(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(StrictValue::String(value))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element()? {
                    values.push(value);
                }
                Ok(StrictValue::Array(values))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate object key"));
                    }
                    values.insert(key, map.next_value()?);
                }
                Ok(StrictValue::Object(values))
            }
        }

        deserializer.deserialize_any(StrictValueVisitor)
    }
}

impl StrictValue {
    fn validate(&self, depth: usize, nodes: &mut usize) -> Result<(), VoiceError> {
        if depth > MAX_JSON_DEPTH {
            return Err(VoiceError::InvalidRequest);
        }
        *nodes = nodes.checked_add(1).ok_or(VoiceError::InvalidRequest)?;
        if *nodes > MAX_JSON_NODES {
            return Err(VoiceError::InvalidRequest);
        }
        match self {
            Self::String(value) if value.len() > MAX_JSON_STRING_BYTES => Err(VoiceError::TooLarge),
            Self::String(_) | Self::Null | Self::Bool(_) | Self::Number(_) => Ok(()),
            Self::Array(values) => {
                for value in values {
                    value.validate(depth + 1, nodes)?;
                }
                Ok(())
            }
            Self::Object(values) => {
                for (key, value) in values {
                    if key.len() > 256 {
                        return Err(VoiceError::TooLarge);
                    }
                    value.validate(depth + 1, nodes)?;
                }
                Ok(())
            }
        }
    }

    fn into_json(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(value) => Value::Bool(value),
            Self::Number(value) => Value::Number(value),
            Self::String(value) => Value::String(value),
            Self::Array(values) => Value::Array(values.into_iter().map(Self::into_json).collect()),
            Self::Object(values) => Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, value.into_json()))
                    .collect::<Map<_, _>>(),
            ),
        }
    }
}

fn decode_request_payload<T>(bytes: &[u8]) -> Result<T, VoiceError>
where
    T: for<'de> Deserialize<'de>,
{
    let value = parse_unique_json(bytes)?;
    let StrictValue::Object(mut root) = value else {
        return Err(VoiceError::InvalidRequest);
    };
    if root.len() != 1 {
        return Err(VoiceError::InvalidRequest);
    }
    let Some(StrictValue::Object(mut request)) = root.remove("request") else {
        return Err(VoiceError::InvalidRequest);
    };
    let Some(StrictValue::Number(version)) = request.remove("protocol_version") else {
        return Err(VoiceError::InvalidRequest);
    };
    if version.as_u64() != Some(u64::from(PUBLIC_PROTOCOL_VERSION)) {
        return Err(VoiceError::InvalidRequest);
    }
    let payload = Value::Object(
        request
            .into_iter()
            .map(|(key, value)| (key, value.into_json()))
            .collect(),
    );
    serde_json::from_value(payload).map_err(|_| VoiceError::InvalidRequest)
}

fn parse_unique_json(bytes: &[u8]) -> Result<StrictValue, VoiceError> {
    if bytes.is_empty() || bytes.len() > MAX_PUBLIC_FRAME_BYTES {
        return Err(if bytes.len() > MAX_PUBLIC_FRAME_BYTES {
            VoiceError::TooLarge
        } else {
            VoiceError::InvalidRequest
        });
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value =
        StrictValue::deserialize(&mut deserializer).map_err(|_| VoiceError::InvalidRequest)?;
    deserializer.end().map_err(|_| VoiceError::InvalidRequest)?;
    let mut nodes = 0;
    value.validate(0, &mut nodes)?;
    Ok(value)
}

pub fn decode_public_request(
    bytes: &[u8],
    command: PublicCommand,
) -> Result<VoiceCommand, VoiceError> {
    macro_rules! decode {
        ($type:ty, $variant:ident) => {
            decode_request_payload::<$type>(bytes).map(|payload| VoiceCommand::$variant(payload))
        };
    }
    match command {
        PublicCommand::GetStatus => decode!(StatusRequest, GetStatus),
        PublicCommand::Start => decode!(StartRequest, Start),
        PublicCommand::Finish => decode!(OwnerRequest, Finish),
        PublicCommand::SubmitText => decode!(SubmitTextRequest, SubmitText),
        PublicCommand::ConfirmTranscript => decode!(ConfirmTranscriptRequest, ConfirmTranscript),
        PublicCommand::Cancel => decode!(CancelRequest, Cancel),
        PublicCommand::ClearSession => {
            decode_request_payload::<EmptyRequest>(bytes).map(|_| VoiceCommand::ClearSession)
        }
        PublicCommand::SelectPreferences => decode!(SelectPreferencesRequest, SelectPreferences),
        PublicCommand::AckIntent => decode!(AckIntentRequest, AckIntent),
        PublicCommand::AssetsGetStatus => {
            decode_request_payload::<EmptyRequest>(bytes).map(|_| VoiceCommand::AssetsGetStatus)
        }
        PublicCommand::AssetsRequestInstall => decode!(AssetInstallRequest, AssetsRequestInstall),
        PublicCommand::AssetsDisable => decode!(AssetRequest, AssetsDisable),
        PublicCommand::AssetsDelete => decode!(AssetRequest, AssetsDelete),
        PublicCommand::AssetsRollback => decode!(AssetRollbackRequest, AssetsRollback),
        PublicCommand::TestOutput => decode!(TestOutputRequest, TestOutput),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDeviceHandle(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDeviceRecord {
    pub handle: InputDeviceHandle,
    pub label: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceCatalog {
    pub revision: u64,
    pub devices: Vec<InputDeviceRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceRecord {
    pub id: VoiceId,
    pub display_name: String,
    pub locale: String,
    pub language: Language,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceCatalog {
    pub revision: u64,
    pub turkish: Vec<VoiceRecord>,
    pub english: Vec<VoiceRecord>,
}
impl VoiceCatalog {
    pub fn get(&self, language: Language) -> Option<&[VoiceRecord]> {
        match language {
            Language::Tr => Some(&self.turkish),
            Language::En => Some(&self.english),
        }
    }
}

#[derive(Debug)]
pub struct OwnedPcm {
    file: File,
    sample_count: u32,
    duration_capped: bool,
}

impl OwnedPcm {
    pub fn new(file: File, sample_count: u32, duration_capped: bool) -> Result<Self, CaptureError> {
        if sample_count == 0 || sample_count > MAX_CAPTURE_SAMPLES {
            return Err(CaptureError::InvalidAudio);
        }
        Ok(Self {
            file,
            sample_count,
            duration_capped,
        })
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub fn duration_capped(&self) -> bool {
        self.duration_capped
    }

    pub fn as_file(&self) -> &File {
        &self.file
    }

    pub fn into_file(self) -> File {
        self.file
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureError {
    Unavailable,
    PermissionDenied,
    DeviceLost,
    UnsupportedFormat,
    Overflow,
    InvalidAudio,
    Cancelled,
    Timeout,
    CleanupFailed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureOperationState {
    PermissionPending,
    Started,
}

#[derive(Debug)]
pub struct CaptureOperation {
    pub owner: VoiceOwner,
    pub operation_id: OperationId,
    pub started_at: Instant,
    pub state: CaptureOperationState,
}

pub trait NativeCaptureProvider: Send {
    fn enumerate_devices(&self) -> Result<InputDeviceCatalog, CaptureError>;
    fn start(
        &mut self,
        context: &TurnContext,
        device: InputDeviceHandle,
    ) -> Result<CaptureOperation, CaptureError>;
    fn finish(&mut self, owner: &VoiceOwner) -> Result<OwnedPcm, CaptureError>;
    fn cancel(&mut self, owner: &VoiceOwner) -> CleanupReceipt;
}

pub(crate) trait LocalAsrProvider: Send {
    fn transcribe(
        &mut self,
        context: &TurnContext,
        pcm: OwnedPcm,
        language_mode: LanguageMode,
        model: crate::voice_authorization::ApprovedModelHandle,
        worker_session: Arc<crate::voice_authorization::AuthorizedWorkerSession>,
    ) -> Result<AsrResult, AsrError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UtteranceSource {
    Typed,
    Speech,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedUtterance {
    pub text: String,
    pub language: Language,
    pub source: UtteranceSource,
    pub eligibility: TranscriptEligibility,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppIntent {
    Overview,
    Conversation,
    Motion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppIntentDescriptor {
    Overview {
        owner: VoiceOwner,
        intent_id: DecimalU64,
    },
    Conversation {
        owner: VoiceOwner,
        intent_id: DecimalU64,
    },
    Motion {
        owner: VoiceOwner,
        intent_id: DecimalU64,
        enabled: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteKind {
    Deterministic,
    LocalModel,
    Clarification,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteOutcome {
    pub kind: RouteKind,
    pub language: Language,
    pub text: String,
    pub intent: Option<AppIntentDescriptor>,
    pub spoken_reply: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouterDecision {
    Resolved(RouteOutcome),
    LanguagePreference {
        language: Language,
        response: RouteOutcome,
    },
    Unhandled {
        utterance: AcceptedUtterance,
        response_language: Option<Language>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationEvent {
    TextDelta {
        owner: VoiceOwner,
        operation_id: OperationId,
        sequence: u64,
        text: String,
    },
    Completed {
        owner: VoiceOwner,
        operation_id: OperationId,
        output_tokens: u16,
    },
    Failed {
        owner: VoiceOwner,
        operation_id: OperationId,
        failure: ConversationFailure,
    },
}

pub(crate) trait LocalConversationProvider: Send {
    fn generate(
        &mut self,
        context: TurnContext,
        utterance: AcceptedUtterance,
        response_language: Language,
        history: &crate::voice::context::CompletedTurnStore,
        model: crate::voice_authorization::ApprovedModelHandle,
        worker_session: Arc<crate::voice_authorization::AuthorizedWorkerSession>,
    ) -> Result<(), ConversationFailure>;
    fn poll(
        &mut self,
        max_delta_bytes: usize,
    ) -> Result<Option<ConversationEvent>, ConversationFailure>;
    fn cancel(&mut self, context: &TurnContext) -> Result<CleanupReceipt, ConversationFailure>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechSegment {
    pub text: String,
    pub language: Language,
    pub voice_id: VoiceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechAcceptance {
    Accepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechError {
    Unavailable,
    WrongLanguage,
    VoiceUnavailable,
    OfflineEvidenceRequired,
    QualityEvidenceRequired,
    TooLong,
    Backpressure,
    Cancelled,
    Timeout,
    Failed,
    CleanupFailed,
}

pub trait LocalSpeechProvider: Send {
    fn enumerate_voices(&self) -> Result<VoiceCatalog, SpeechError>;
    fn available_credits(&self) -> u8;
    fn speak(
        &mut self,
        context: &TurnContext,
        segment: SpeechSegment,
    ) -> Result<SpeechAcceptance, SpeechError>;
    fn cancel(&mut self, context: &TurnContext) -> Result<CleanupReceipt, SpeechError>;
    fn unload(&mut self) -> Result<CleanupReceipt, SpeechError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeCode {
    NoSpeech,
    TranscriptNeedsReview,
    TranscriptOverflowReview,
    UnsupportedLanguage,
    ModelUnavailable,
    SpeechUnavailable,
    PartialAnswer,
    NotSpoken,
    CleanupPending,
    TailGuard,
    ConfirmationRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceCatalogRevision {
    #[serde(with = "decimal_u64")]
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputProgress {
    pub queued_segments: u8,
    pub completed_segments: u8,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceSnapshot {
    pub metadata: VoiceSnapshotMetadata,
    pub owner: Option<VoiceOwner>,
    pub history: Vec<HistoryMessage>,
    pub pending: Vec<PendingText>,
    pub input_devices: Vec<InputDeviceRecord>,
    pub voices: Vec<VoiceRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryMessage {
    pub id: String,
    pub role: MessageRole,
    pub language: Language,
    pub source: UtteranceSource,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingText {
    pub stream: PendingStream,
    #[serde(with = "decimal_u64")]
    pub sequence: u64,
    pub language: Language,
    pub source: UtteranceSource,
    pub text: String,
    pub final_chunk: bool,
    pub partial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingStream {
    Transcript,
    Answer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotBegin {
    pub protocol_version: u8,
    pub event_sequence: DecimalU64,
    pub resync_id: DecimalU64,
    pub revision: DecimalU64,
    pub owner: Option<VoiceOwner>,
    pub page_count: u16,
    pub total_text_bytes: u32,
    pub total_metadata_bytes: u32,
    pub digest_sha256: String,
    pub metadata: VoiceSnapshotMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotPage {
    pub protocol_version: u8,
    pub event_sequence: DecimalU64,
    pub resync_id: DecimalU64,
    pub revision: DecimalU64,
    pub owner: Option<VoiceOwner>,
    pub index: u16,
    pub section: SnapshotSection,
    pub items: Vec<SnapshotPageItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSection {
    History,
    Pending,
    InputDevices,
    Voices,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SnapshotPageItem {
    HistoryMessage {
        message_index: u16,
        id: String,
        role: MessageRole,
        language: Language,
        source: UtteranceSource,
        total_bytes: u32,
        chunk_count: u16,
    },
    HistoryChunk {
        message_index: u16,
        byte_offset: u32,
        total_bytes: u32,
        chunk_index: u16,
        chunk_count: u16,
        text: String,
    },
    PendingChunk {
        stream: PendingStream,
        #[serde(with = "decimal_u64")]
        sequence: u64,
        language: Language,
        source: UtteranceSource,
        byte_offset: u32,
        total_bytes: u32,
        chunk_index: u16,
        chunk_count: u16,
        final_chunk: bool,
        partial: bool,
        text: String,
    },
    InputDevice {
        record: InputDeviceRecord,
    },
    Voice {
        record: VoiceRecord,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotEnd {
    pub protocol_version: u8,
    pub event_sequence: DecimalU64,
    pub resync_id: DecimalU64,
    pub revision: DecimalU64,
    pub owner: Option<VoiceOwner>,
    pub page_count: u16,
    pub total_text_bytes: u32,
    pub total_metadata_bytes: u32,
    pub digest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "frame", rename_all = "snake_case", deny_unknown_fields)]
pub enum SnapshotFrame {
    Begin(Box<SnapshotBegin>),
    Page(Box<SnapshotPage>),
    End(SnapshotEnd),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotTransferHeader {
    pub resync_id: DecimalU64,
    pub event_sequence: DecimalU64,
    pub revision: DecimalU64,
    pub owner: Option<VoiceOwner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceResultCode {
    Accepted,
    Pending,
    Completed,
    NoChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceCommandWireResult {
    pub accepted: bool,
    pub code: VoiceResultCode,
    pub transfer: Option<SnapshotTransferHeader>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceEventWire {
    pub protocol_version: u8,
    pub event_sequence: DecimalU64,
    pub revision: DecimalU64,
    pub owner: Option<VoiceOwner>,
    pub event: VoiceEventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceEventBatch {
    pub protocol_version: u8,
    pub after_event_sequence: DecimalU64,
    pub latest_event_sequence: DecimalU64,
    pub resync_required: bool,
    pub events: Vec<VoiceEventWire>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureLifecycleState {
    PermissionPending,
    Started,
    PermissionDenied,
    DeviceLost,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoiceEventKind {
    StateChanged {
        phase: VoicePhase,
        route_kind: Option<RouteKind>,
        cancel_available: bool,
        cleanup: CleanupReceipt,
    },
    CaptureLifecycle {
        state: CaptureLifecycleState,
    },
    TranscriptDelta {
        #[serde(with = "decimal_u64")]
        sequence: u64,
        text: String,
        final_chunk: bool,
        language: Option<Language>,
        source: UtteranceSource,
    },
    AnswerDelta {
        #[serde(with = "decimal_u64")]
        sequence: u64,
        text: String,
        final_chunk: bool,
        language: Language,
        partial: bool,
    },
    CaptureMeter {
        elapsed_ms: u32,
        level: f32,
    },
    Intent {
        descriptor: AppIntentDescriptor,
    },
    Notice {
        code: NoticeCode,
    },
    Failure {
        code: VoiceError,
    },
    OutputStarted,
    OutputDrained,
    Terminal {
        receipt: CleanupReceipt,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingVoiceEvent {
    pub owner: Option<VoiceOwner>,
    pub operation_id: Option<OperationId>,
    pub event: VoiceEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventQueueError {
    Full,
    Closed,
    SequenceExhausted,
}

pub trait VoiceEventSender: Send + Sync {
    fn try_send(&self, event: PendingVoiceEvent) -> Result<(), EventQueueError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetError {
    NotFound,
    WrongPurpose,
    Unavailable,
    ApprovalRequired,
    InvalidBinding,
    DigestMismatch,
    StorageFailure,
    Cancelled,
    Timeout,
    CleanupFailed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetCatalog {
    pub revision: DecimalU64,
    pub assets: Vec<AssetStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetStatus {
    pub asset_id: String,
    pub purpose: AssetPurpose,
    pub status: AssetState,
    pub code: AssetStatusCode,
    pub asset_disclosure: AssetDisclosure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetDisclosure {
    pub candidate_name: String,
    pub version: String,
    pub source_origin: String,
    pub source_revision: String,
    pub purpose: AssetPurpose,
    pub expected_bytes: DecimalU64,
    pub digest_sha256: String,
    pub license_ids: Vec<String>,
    pub notice_ids: Vec<String>,
    pub max_staging_bytes: DecimalU64,
    pub max_retained_bytes: DecimalU64,
    pub max_disk_bytes: DecimalU64,
    pub redirect_origins: Vec<String>,
    pub retention: AssetRetention,
    pub build: Option<AssetBuildDisclosure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSourceInputDisclosure {
    pub name: String,
    pub origin: String,
    pub revision: String,
    pub source_tree_id: String,
    pub maximum_transfer_bytes: DecimalU64,
    pub observed_bytes: Option<DecimalU64>,
    pub archive_sha256: Option<String>,
    pub digest_status: AssetDigestStatus,
    pub license_ids: Vec<String>,
    pub notice_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetDigestStatus {
    NotPublished,
    ObservedUnreviewed,
    IndependentlyVerified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetBuildDisclosure {
    pub source_inputs: Vec<AssetSourceInputDisclosure>,
    pub toolchain_identity: String,
    pub recipe_identity: String,
    pub maximum_workspace_bytes: DecimalU64,
    pub maximum_output_bytes: DecimalU64,
    pub cpu_limit: u16,
    pub memory_limit_bytes: DecimalU64,
    pub deadline_seconds: u32,
    pub network_policy: AssetNetworkPolicy,
    pub write_policy: AssetWritePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetNetworkPolicy {
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetWritePolicy {
    AppOwnedBuildWorkspaceOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetRetention {
    AppManagedUntilExplicitRemoval,
    BuildInputsRetainedForProvenance,
    RemovedAfterPromotion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetState {
    Blocked,
    AwaitingApproval,
    Ready,
    Disabled,
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetStatusCode {
    NotReviewed,
    SourceReviewRequired,
    RuntimeBuildRequired,
    OwnerTrustSetupRequired,
    ModelApprovalRequired,
    TransferNotGranted,
    AssetMissing,
    Verified,
    Disabled,
    RecoveryRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetInstallOutcome {
    pub accepted: bool,
    pub status: AssetStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRemovalOutcome {
    pub removed: bool,
    pub catalog: AssetCatalog,
}

pub trait AssetManager: Send + Sync {
    fn inventory(&self) -> Result<AssetCatalog, AssetError>;
    fn request_install(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
        asset_id: &str,
        cancellation: &CancelToken,
    ) -> Result<AssetInstallOutcome, AssetError>;
    fn disable(&self, asset_id: &str) -> Result<AssetCatalog, AssetError>;
    fn delete(&self, asset_id: &str) -> Result<AssetRemovalOutcome, AssetError>;
    fn rollback(&self, asset_id: &str, version_id: &str) -> Result<AssetCatalog, AssetError>;
    fn open_model(
        &self,
        asset_id: &str,
        purpose: AssetPurpose,
    ) -> Result<crate::voice_authorization::ApprovedModelHandle, AssetError>;
    fn runtime_closure(
        &self,
        asset_id: &str,
    ) -> Result<crate::voice_authorization::ApprovedRuntimeClosure, AssetError>;
}

// This deliberately does not implement Deserialize or Serialize: native grants and model FDs
// must never be expressible in the public IPC schema.
#[derive(Debug)]
pub struct NativeVoiceGrant {
    pub(crate) generation: u64,
}

pub trait VoiceAuthority: Send + Sync {
    fn authorize(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
        command: &VoiceCommand,
    ) -> Result<NativeVoiceGrant, VoiceError>;
}

pub trait VoiceConsentPrompt: Send + Sync {
    fn confirm(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
        disclosure: &AssetDisclosure,
        cancellation: &CancelToken,
    ) -> Result<crate::voice_authorization::OwnerConsent, AssetError>;
}

pub(crate) trait VoiceAssetAuthority: Send + Sync {
    fn begin_worker_authorization_session(
        &self,
        cancellation: &CancelToken,
        deadline: Instant,
        needs: crate::voice_authorization::WorkerAuthorizationNeeds,
    ) -> Result<
        Arc<crate::voice_authorization::AuthorizedWorkerSession>,
        crate::voice_authorization::VoiceAuthorizationError,
    >;
    fn authorize_install(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
        asset_id: &str,
        disclosure: &AssetDisclosure,
        consent: crate::voice_authorization::OwnerConsent,
        cancellation: &CancelToken,
    ) -> Result<
        crate::voice_authorization::VoiceGrant,
        crate::voice_authorization::VoiceAuthorizationError,
    >;
    fn authorize_model_purpose(
        &self,
        caller: &crate::voice::ipc::NativeCallerContext,
        candidate: &crate::voice_authorization::VerifiedModelFile,
        cancellation: &CancelToken,
    ) -> Result<(), crate::voice_authorization::VoiceAuthorizationError>;
    fn begin_transfer(
        &self,
        grant: crate::voice_authorization::VoiceGrant,
        cancellation: &CancelToken,
    ) -> Result<
        crate::voice_authorization::VoiceTransferLease,
        crate::voice_authorization::VoiceAuthorizationError,
    >;
    fn finish_transfer(
        &self,
        lease: crate::voice_authorization::VoiceTransferLease,
        result: crate::voice_authorization::TransferResult,
        cancellation: &CancelToken,
    ) -> Result<(), crate::voice_authorization::VoiceAuthorizationError>;
}
