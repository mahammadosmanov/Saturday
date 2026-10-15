export type DecimalU64 = string;
export type VoiceLanguage = "tr" | "en";
export type VoiceLanguageMode = VoiceLanguage | "auto";

const MAX_U64 = 18_446_744_073_709_551_615n;
const CANONICAL_U64 = /^(?:0|[1-9][0-9]*)$/;
const encoder = new TextEncoder();

export function isDecimalU64(value: unknown): value is DecimalU64 {
	return (
		typeof value === "string" &&
		CANONICAL_U64.test(value) &&
		BigInt(value) <= MAX_U64
	);
}

export function compareDecimalU64(left: DecimalU64, right: DecimalU64): -1 | 0 | 1 {
	if (!isDecimalU64(left) || !isDecimalU64(right))
		throw new TypeError("Expected canonical decimal u64 strings");
	const a = BigInt(left);
	const b = BigInt(right);
	return a < b ? -1 : a > b ? 1 : 0;
}

export type SequenceRelation = "next" | "duplicate" | "stale" | "gap";

export function sequenceRelation(
	last: DecimalU64,
	next: DecimalU64,
): SequenceRelation {
	const order = compareDecimalU64(next, last);
	if (order === 0) return "duplicate";
	if (order < 0) return "stale";
	return BigInt(next) === BigInt(last) + 1n ? "next" : "gap";
}

export interface VoiceOwner {
	session_id: string;
	turn_id: string;
	generation: DecimalU64;
}

export function sameVoiceOwner(left: VoiceOwner | null, right: VoiceOwner | null): boolean {
	return (
		left === right ||
		(left !== null &&
			right !== null &&
			left.session_id === right.session_id &&
			left.turn_id === right.turn_id &&
			left.generation === right.generation)
	);
}

export type VoicePhase =
	| "unavailable"
	| "setup_required"
	| "idle"
	| "requesting_permission"
	| "listening"
	| "transcribing"
	| "needs_review"
	| "routing"
	| "thinking"
	| "speaking"
	| "cancelling"
	| "error";

export type ReadinessState = "unavailable" | "setup_required" | "ready";
export type ReadinessCode =
	| "not_inspected"
	| "source_review_required"
	| "runtime_build_required"
	| "asset_approval_required"
	| "owner_trust_setup_required"
	| "provider_unavailable"
	| "permission_required"
	| "offline_evidence_required"
	| "quality_evidence_required"
	| "ready";

export interface ProviderReadiness {
	state: ReadinessState;
	code: ReadinessCode;
}

export interface VoiceReadiness {
	capture: ProviderReadiness;
	asr: ProviderReadiness;
	router: ProviderReadiness;
	llm: ProviderReadiness;
	speech: ProviderReadiness;
	routine_tasks: { completed: DecimalU64; without_llm: DecimalU64 };
	product_ready: boolean;
}

export interface VoicePreferences {
	language_mode: VoiceLanguageMode | null;
	input_device_handle: string | null;
	voice_ids: { tr: string | null; en: string | null };
	spoken_reply: boolean;
	review_before_send: boolean;
}

export type VoiceErrorCode =
	| "invalid_request"
	| "unauthorized"
	| "unavailable"
	| "setup_required"
	| "busy"
	| "stale_owner"
	| "stale_catalog"
	| "unsupported"
	| "permission_denied"
	| "too_large"
	| "timeout"
	| "cancelled"
	| "provider_failed"
	| "protocol_error"
	| "sequence_exhausted"
	| "transfer_expired"
	| "cleanup_failed"
	| "internal";

export type VoiceNoticeCode =
	| "no_speech"
	| "transcript_needs_review"
	| "transcript_overflow_review"
	| "unsupported_language"
	| "model_unavailable"
	| "speech_unavailable"
	| "partial_answer"
	| "not_spoken"
	| "cleanup_pending"
	| "tail_guard"
	| "confirmation_required";

export type CleanupCode =
	| "no_resources_owned"
	| "capture_stopped"
	| "worker_exited"
	| "speech_stopped"
	| "verified"
	| "failed";

export interface CleanupReceipt {
	stopped: boolean;
	verified: boolean;
	code: CleanupCode;
}

export type RouteKind = "deterministic" | "local_model" | "clarification" | "unavailable";
export type MessageRole = "user" | "assistant";
export type UtteranceSource = "typed" | "speech";
export type PendingStream = "transcript" | "answer";

export interface VoiceInputDevice {
	handle: string;
	label: string;
	is_default: boolean;
}

export interface VoiceCatalogRecord {
	id: string;
	display_name: string;
	locale: string;
	language: VoiceLanguage;
}

export interface VoiceSnapshotMetadata {
	session_id: string;
	phase: VoicePhase;
	preferences: VoicePreferences;
	resolved_language: VoiceLanguage | null;
	readiness: VoiceReadiness;
	voice_catalog: { revision: DecimalU64 };
	cancel_available: boolean;
	cleanup: CleanupReceipt;
	route_kind: RouteKind | null;
	capture_elapsed_ms: number | null;
	output_progress: { queued_segments: number; completed_segments: number; active: boolean };
	next_transcript_sequence: DecimalU64;
	next_answer_sequence: DecimalU64;
	notices: VoiceNoticeCode[];
	error: VoiceErrorCode | null;
}

export interface VoiceHistoryMessage {
	id: string;
	role: MessageRole;
	language: VoiceLanguage;
	source: UtteranceSource;
	text: string;
}

export interface VoicePendingText {
	stream: PendingStream;
	sequence: DecimalU64;
	language: VoiceLanguage;
	source: UtteranceSource;
	text: string;
	final_chunk: boolean;
	partial: boolean;
}

export interface VoiceSnapshot {
	revision: DecimalU64;
	event_sequence: DecimalU64;
	metadata: VoiceSnapshotMetadata;
	owner: VoiceOwner | null;
	history: VoiceHistoryMessage[];
	pending: VoicePendingText[];
	input_devices: VoiceInputDevice[];
	voices: VoiceCatalogRecord[];
}

export type AssetPurpose = "runtime" | "asr_weights" | "llm_weights" | "portable_voice";
export type AssetRetention =
	| "app_managed_until_explicit_removal"
	| "build_inputs_retained_for_provenance"
	| "removed_after_promotion";
export type AssetState =
	| "blocked"
	| "awaiting_approval"
	| "ready"
	| "disabled"
	| "recovery_required";
export type AssetStatusCode =
	| "not_reviewed"
	| "source_review_required"
	| "runtime_build_required"
	| "model_approval_required"
	| "owner_trust_setup_required"
	| "transfer_not_granted"
	| "asset_missing"
	| "verified"
	| "disabled"
	| "recovery_required";
export type AssetDigestStatus =
	| "not_published"
	| "observed_unreviewed"
	| "independently_verified";
export interface AssetSourceInputDisclosure {
	name: string;
	origin: string;
	revision: string;
	source_tree_id: string;
	maximum_transfer_bytes: DecimalU64;
	observed_bytes: DecimalU64 | null;
	archive_sha256: string | null;
	digest_status: AssetDigestStatus;
	license_ids: string[];
	notice_ids: string[];
}
export interface AssetBuildDisclosure {
	source_inputs: AssetSourceInputDisclosure[];
	toolchain_identity: string;
	recipe_identity: string;
	maximum_workspace_bytes: DecimalU64;
	maximum_output_bytes: DecimalU64;
	cpu_limit: number;
	memory_limit_bytes: DecimalU64;
	deadline_seconds: number;
	network_policy: "none";
	write_policy: "app_owned_build_workspace_only";
}
export interface AssetDisclosure {
	candidate_name: string;
	version: string;
	source_origin: string;
	source_revision: string;
	purpose: AssetPurpose;
	expected_bytes: DecimalU64;
	digest_sha256: string;
	license_ids: string[];
	notice_ids: string[];
	max_staging_bytes: DecimalU64;
	max_retained_bytes: DecimalU64;
	max_disk_bytes: DecimalU64;
	redirect_origins: string[];
	retention: AssetRetention;
	build: AssetBuildDisclosure | null;
}
export interface AssetStatus {
	asset_id: string;
	purpose: AssetPurpose;
	status: AssetState;
	code: AssetStatusCode;
	asset_disclosure: AssetDisclosure;
}
export interface AssetCatalog {
	revision: DecimalU64;
	assets: AssetStatus[];
}
export interface AssetInstallOutcome {
	accepted: boolean;
	status: AssetStatus;
}
export interface AssetRemovalOutcome {
	removed: boolean;
	catalog: AssetCatalog;
}

export type AppIntentDescriptor =
	| { intent: "overview"; owner: VoiceOwner; intent_id: DecimalU64 }
	| { intent: "conversation"; owner: VoiceOwner; intent_id: DecimalU64 }
	| { intent: "motion"; owner: VoiceOwner; intent_id: DecimalU64; enabled: boolean };

export type VoiceCaptureLifecycleState =
	| "permission_pending"
	| "started"
	| "permission_denied"
	| "device_lost"
	| "cancelled"
	| "failed";

export type VoiceEventKind =
	| {
			type: "state_changed";
			phase: VoicePhase;
			route_kind: RouteKind | null;
			cancel_available: boolean;
			cleanup: CleanupReceipt;
		}
	| { type: "capture_lifecycle"; state: VoiceCaptureLifecycleState }
	| {
			type: "transcript_delta";
			sequence: DecimalU64;
			text: string;
			final_chunk: boolean;
			language: VoiceLanguage | null;
			source: UtteranceSource;
		}
	| {
			type: "answer_delta";
			sequence: DecimalU64;
			text: string;
			final_chunk: boolean;
			language: VoiceLanguage;
			partial: boolean;
		}
	| { type: "capture_meter"; elapsed_ms: number; level: number }
	| { type: "intent"; descriptor: AppIntentDescriptor }
	| { type: "notice"; code: VoiceNoticeCode }
	| { type: "failure"; code: VoiceErrorCode }
	| { type: "output_started" }
	| { type: "output_drained" }
	| { type: "terminal"; receipt: CleanupReceipt };
export interface VoiceReducerState {
	snapshot: VoiceSnapshot;
	event_sequence: DecimalU64;
	meter: { elapsed_ms: number; level: number } | null;
	capture_lifecycle: VoiceCaptureLifecycleState | null;
	intents: AppIntentDescriptor[];
	transcript_deltas: VoiceLiveText[];
	answer_deltas: VoiceLiveText[];
	live_text_bytes: number;
}
export interface VoiceEventWire {
	protocol_version: 1;
	event_sequence: DecimalU64;
	revision: DecimalU64;
	owner: VoiceOwner | null;
	event: VoiceEventKind;
}

export interface VoiceLiveText {
	stream: PendingStream;
	sequence: DecimalU64;
	language: VoiceLanguage | null;
	source: UtteranceSource;
	text: string;
	final_chunk: boolean;
	partial: boolean;
}


export function initialVoiceReducerState(snapshot: VoiceSnapshot): VoiceReducerState {
	return {
		snapshot,
		event_sequence: snapshot.event_sequence,
		meter: null,
		capture_lifecycle: null,
		intents: [],
		transcript_deltas: [],
		answer_deltas: [],
		live_text_bytes: 0,
	};
}

export type VoiceReductionOutcome = "applied" | "ignored" | "resync";
export interface VoiceReduction {
	outcome: VoiceReductionOutcome;
	state: VoiceReducerState;
}

function ownerMatchesForEvent(state: VoiceReducerState, event: VoiceEventWire): boolean {
	const current = state.snapshot.owner;
	if (sameVoiceOwner(current, event.owner)) return true;
	if (current !== null && event.owner === null && event.event.type === "state_changed")
		return ["idle", "unavailable", "setup_required", "error"].includes(event.event.phase);
	if (current === null && event.owner !== null && event.event.type === "state_changed")
		return state.snapshot.metadata.phase === "idle";
	return false;
}

function advance(state: VoiceReducerState, event: VoiceEventWire): VoiceReducerState {
	return {
		...state,
		event_sequence: event.event_sequence,
		snapshot: {
			...state.snapshot,
			revision: event.revision,
			event_sequence: event.event_sequence,
		},
	};
}

export function reduceVoiceEvent(
	state: VoiceReducerState,
	wire: VoiceEventWire,
): VoiceReduction {
	const sequence = sequenceRelation(state.event_sequence, wire.event_sequence);
	if (sequence === "gap") return { outcome: "resync", state };
	if (sequence === "duplicate" || sequence === "stale")
		return { outcome: "ignored", state };
	if (compareDecimalU64(wire.revision, state.snapshot.revision) < 0)
		return { outcome: "ignored", state: advance(state, wire) };
	if (!ownerMatchesForEvent(state, wire))
		return { outcome: "ignored", state: advance(state, wire) };

	const next = advance(state, wire);
	const metadata = next.snapshot.metadata;
	switch (wire.event.type) {
		case "state_changed":
			next.snapshot = {
				...next.snapshot,
				owner: wire.owner,
				metadata: {
					...metadata,
					phase: wire.event.phase,
					route_kind: wire.event.route_kind,
					cancel_available: wire.event.cancel_available,
					cleanup: wire.event.cleanup,
				},
			};
			break;
		case "capture_lifecycle":
			next.capture_lifecycle = wire.event.state;
			break;
		case "transcript_delta": {
			if (wire.event.sequence !== metadata.next_transcript_sequence)
				return { outcome: "resync", state };
			const bytes = encoder.encode(wire.event.text).byteLength;
			const following = BigInt(wire.event.sequence) + 1n;
			if (
				following > MAX_U64 ||
				next.transcript_deltas.length >= 64 ||
				next.live_text_bytes + bytes > 81_728
			)
				return { outcome: "resync", state };
			next.snapshot = {
				...next.snapshot,
				metadata: { ...metadata, next_transcript_sequence: following.toString() },
			};
			next.transcript_deltas = [
				...state.transcript_deltas,
				{
					stream: "transcript",
					sequence: wire.event.sequence,
					language: wire.event.language,
					source: wire.event.source,
					text: wire.event.text,
					final_chunk: wire.event.final_chunk,
					partial: false,
				},
			];
			next.live_text_bytes += bytes;
			break;
		}
		case "answer_delta": {
			if (wire.event.sequence !== metadata.next_answer_sequence)
				return { outcome: "resync", state };
			const bytes = encoder.encode(wire.event.text).byteLength;
			const following = BigInt(wire.event.sequence) + 1n;
			if (
				following > MAX_U64 ||
				next.answer_deltas.length >= 64 ||
				next.live_text_bytes + bytes > 81_728
			)
				return { outcome: "resync", state };
			next.snapshot = {
				...next.snapshot,
				metadata: { ...metadata, next_answer_sequence: following.toString() },
			};
			next.answer_deltas = [
				...state.answer_deltas,
				{
					stream: "answer",
					sequence: wire.event.sequence,
					language: wire.event.language,
					source: "typed",
					text: wire.event.text,
					final_chunk: wire.event.final_chunk,
					partial: wire.event.partial,
				},
			];
			next.live_text_bytes += bytes;
			break;
		}
		case "capture_meter":
			next.meter = { elapsed_ms: wire.event.elapsed_ms, level: wire.event.level };
			next.snapshot = {
				...next.snapshot,
				metadata: { ...metadata, capture_elapsed_ms: wire.event.elapsed_ms },
			};
			break;
		case "intent":
			if (next.intents.length >= 64) return { outcome: "resync", state };
			next.intents = [...state.intents, wire.event.descriptor];
			break;
		case "notice":
			next.snapshot = {
				...next.snapshot,
				metadata: {
					...metadata,
					notices: metadata.notices.includes(wire.event.code)
						? metadata.notices
						: [...metadata.notices, wire.event.code],
				},
			};
			break;
		case "failure":
			next.snapshot = {
				...next.snapshot,
				metadata: { ...metadata, error: wire.event.code },
			};
			break;
		case "output_started":
			next.snapshot = {
				...next.snapshot,
				metadata: {
					...metadata,
					output_progress: { ...metadata.output_progress, active: true },
				},
			};
			break;
		case "output_drained":
			next.snapshot = {
				...next.snapshot,
				metadata: {
					...metadata,
					output_progress: { ...metadata.output_progress, active: false },
				},
			};
			break;
		case "terminal":
			next.snapshot = {
				...next.snapshot,
				metadata: { ...metadata, cleanup: wire.event.receipt },
			};
			break;
	}
	return { outcome: "applied", state: next };
}

export type VoiceTextFailure =
	| "empty"
	| "invalid_unicode"
	| "too_many_scalars"
	| "too_many_bytes";

export type VoiceTextValidation =
	| { ok: true; scalar_count: number; utf8_bytes: number }
	| { ok: false; reason: VoiceTextFailure };

function scalarCount(text: string): number | null {
	let scalars = 0;
	for (let index = 0; index < text.length; index += 1) {
		const current = text.charCodeAt(index);
		if (current >= 0xd800 && current <= 0xdbff) {
			const next = text.charCodeAt(index + 1);
			if (!(next >= 0xdc00 && next <= 0xdfff)) return null;
			index += 1;
		} else if (current >= 0xdc00 && current <= 0xdfff) {
			return null;
		}
		scalars += 1;
	}
	return scalars;
}

function validateText(
	text: string,
	maxScalars: number,
	maxBytes: number,
): VoiceTextValidation {
	const scalar_count = scalarCount(text);
	if (scalar_count === null) return { ok: false, reason: "invalid_unicode" };
	if (!text.trim()) return { ok: false, reason: "empty" };
	if (scalar_count > maxScalars)
		return { ok: false, reason: "too_many_scalars" };
	const utf8_bytes = encoder.encode(text).byteLength;
	if (utf8_bytes > maxBytes) return { ok: false, reason: "too_many_bytes" };
	return { ok: true, scalar_count, utf8_bytes };
}

export function validateTypedText(text: string): VoiceTextValidation {
	return validateText(text, 500, 2_000);
}

export function validateReviewedTranscript(text: string): VoiceTextValidation {
	return validateText(text, 2_000, 8_000);
}

export function validateAnswerText(text: string): VoiceTextValidation {
	return validateText(text, 2_000, 8_192);
}
