import {
	compareDecimalU64,
	isDecimalU64,
	type AppIntentDescriptor,
	type AssetBuildDisclosure,
	type AssetCatalog,
	type AssetDisclosure,
	type AssetDigestStatus,
	type AssetInstallOutcome,
	type AssetPurpose,
	type AssetRemovalOutcome,
	type AssetRetention,
	type AssetSourceInputDisclosure,
	type AssetState,
	type AssetStatus,
	type AssetStatusCode,
	type CleanupCode,
	type CleanupReceipt,
	type DecimalU64,
	type MessageRole,
	type PendingStream,
	type ProviderReadiness,
	type ReadinessCode,
	type ReadinessState,
	type RouteKind,
	type UtteranceSource,
	type VoiceCaptureLifecycleState,
	type VoiceCatalogRecord,
	type VoiceErrorCode,
	type VoiceEventKind,
	type VoiceEventWire,
	type VoiceHistoryMessage,
	type VoiceInputDevice,
	type VoiceLanguage,
	type VoiceLanguageMode,
	type VoiceNoticeCode,
	type VoiceOwner,
	type VoicePhase,
	type VoicePreferences,
	type VoiceReadiness,
	type VoiceSnapshot,
	type VoiceSnapshotMetadata,
	type VoicePendingText,
} from "../domain/voice";
import { sequenceRelation } from "../domain/voice";

const encoder = new TextEncoder();
const MAX_FRAME_BYTES = 65_536;
const MAX_PAGES = 128;
const MAX_PAGE_TEXT_BYTES = 4_096;
const MAX_PAGE_METADATA_BYTES = 48 * 1024;
const MAX_SNAPSHOT_TEXT_BYTES = 81_728;
const MAX_SNAPSHOT_METADATA_BYTES = 256 * 1024;
const MAX_HISTORY_MESSAGES = 40;
const MAX_HISTORY_BYTES = 65_536;
const MAX_DEVICES = 64;
const MAX_VOICES = 128;
const MAX_EVENT_BATCH = 16;
const MAX_TEXT_DELTA_BYTES = 4_096;
const MAX_ASSET_LIST_ITEMS = 8_192;


export class VoiceProtocolError extends Error {
	constructor() {
		super("Native voice protocol data was invalid");
		this.name = "VoiceProtocolError";
	}
}

function fail(): never {
	throw new VoiceProtocolError();
}

function isRecord(value: unknown): value is Record<string, unknown> {
	if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
	const prototype = Object.getPrototypeOf(value);
	return prototype === Object.prototype || prototype === null;
}

function exactRecord(value: unknown, keys: readonly string[]): Record<string, unknown> {
	if (!isRecord(value)) return fail();
	const actual = Object.keys(value);
	if (actual.length !== keys.length || keys.some((key) => !Object.hasOwn(value, key))) return fail();
	return value;
}

function integer(value: unknown, maximum: number): number {
	if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0 || value > maximum)
		return fail();
	return value;
}

function boolean(value: unknown): boolean {
	if (typeof value !== "boolean") return fail();
	return value;
}

function validScalars(value: string): boolean {
	for (let index = 0; index < value.length; index += 1) {
		const current = value.charCodeAt(index);
		if (current >= 0xd800 && current <= 0xdbff) {
			const next = value.charCodeAt(index + 1);
			if (!(next >= 0xdc00 && next <= 0xdfff)) return false;
			index += 1;
		} else if (current >= 0xdc00 && current <= 0xdfff) {
			return false;
		}
	}
	return true;
}

function text(value: unknown, maxBytes: number, allowEmpty = true): string {
	if (typeof value !== "string" || !validScalars(value) || (!allowEmpty && value.length === 0))
		return fail();
	if (encoder.encode(value).byteLength > maxBytes) return fail();
	return value;
}

function decimal(value: unknown): DecimalU64 {
	if (!isDecimalU64(value)) return fail();
	return value;
}

function oneOf<const Values extends readonly string[]>(value: unknown, values: Values): Values[number] {
	if (typeof value !== "string" || !values.includes(value)) return fail();
	return value as Values[number];
}

function nullable<T>(value: unknown, parse: (item: unknown) => T): T | null {
	return value === null ? null : parse(value);
}

function list<T>(value: unknown, maxLength: number, parse: (item: unknown) => T): T[] {
	if (!Array.isArray(value) || value.length > maxLength) return fail();
	return value.map(parse);
}

function containsControlCharacter(value: string): boolean {
	for (let index = 0; index < value.length; index += 1) {
		const codeUnit = value.charCodeAt(index);
		if (codeUnit <= 0x1f || codeUnit === 0x7f) return true;
	}
	return false;
}

function owner(value: unknown): VoiceOwner | null {
	if (value === null) return null;
	const record = exactRecord(value, ["session_id", "turn_id", "generation"]);
	const session_id = text(record.session_id, 128, false);
	const turn_id = text(record.turn_id, 128, false);
	if (containsControlCharacter(session_id) || containsControlCharacter(turn_id)) fail();
	return { session_id, turn_id, generation: decimal(record.generation) };
}

const LANGUAGES = ["tr", "en"] as const;
const LANGUAGE_MODES = ["tr", "en", "auto"] as const;
const PHASES = [
	"unavailable", "setup_required", "idle", "requesting_permission", "listening",
	"transcribing", "needs_review", "routing", "thinking", "speaking", "cancelling", "error",
] as const;
const READINESS_STATES = ["unavailable", "setup_required", "ready"] as const;
const READINESS_CODES = [
	"not_inspected", "source_review_required", "runtime_build_required", "asset_approval_required",
	"owner_trust_setup_required", "asset_unavailable", "provider_unavailable", "permission_required",
	"offline_evidence_required", "quality_evidence_required", "ready",
] as const;
const ERRORS = [
	"invalid_request", "unauthorized", "unavailable", "setup_required", "busy", "stale_owner",
	"stale_catalog", "unsupported", "permission_denied", "too_large", "timeout", "cancelled",
	"provider_failed", "protocol_error", "sequence_exhausted", "transfer_expired", "cleanup_failed",
	"internal",
] as const;
const NOTICES = [
	"no_speech", "transcript_needs_review", "transcript_overflow_review", "unsupported_language",
	"model_unavailable", "speech_unavailable", "partial_answer", "not_spoken", "cleanup_pending",
	"tail_guard", "confirmation_required",
] as const;
const CLEANUP_CODES = [
	"no_resources_owned", "capture_stopped", "worker_exited", "speech_stopped", "verified", "failed",
] as const;
const ROUTES = ["deterministic", "local_model", "clarification", "unavailable"] as const;
const ROLES = ["user", "assistant"] as const;
const SOURCES = ["typed", "speech"] as const;
const STREAMS = ["transcript", "answer"] as const;

function language(value: unknown): VoiceLanguage {
	return oneOf(value, LANGUAGES);
}

function languageMode(value: unknown): VoiceLanguageMode {
	return oneOf(value, LANGUAGE_MODES);
}

function preferences(value: unknown): VoicePreferences {
	const record = exactRecord(value, [
		"language_mode", "input_device_handle", "voice_ids", "spoken_reply", "review_before_send",
	]);
	const voiceIds = exactRecord(record.voice_ids, ["tr", "en"]);
	return {
		language_mode: nullable(record.language_mode, languageMode),
		input_device_handle: nullable(record.input_device_handle, (item) => text(item, 8_192, false)),
		voice_ids: {
			tr: nullable(voiceIds.tr, (item) => text(item, 8_192, false)),
			en: nullable(voiceIds.en, (item) => text(item, 8_192, false)),
		},
		spoken_reply: boolean(record.spoken_reply),
		review_before_send: boolean(record.review_before_send),
	};
}

function cleanup(value: unknown): CleanupReceipt {
	const record = exactRecord(value, ["stopped", "verified", "code"]);
	return {
		stopped: boolean(record.stopped),
		verified: boolean(record.verified),
		code: oneOf(record.code, CLEANUP_CODES) as CleanupCode,
	};
}

function providerReadiness(value: unknown): ProviderReadiness {
	const record = exactRecord(value, ["state", "code"]);
	return {
		state: oneOf(record.state, READINESS_STATES) as ReadinessState,
		code: oneOf(record.code, READINESS_CODES) as ReadinessCode,
	};
}

function readiness(value: unknown): VoiceReadiness {
	const record = exactRecord(value, ["capture", "asr", "router", "llm", "speech", "routine_tasks", "product_ready"]);
	const tasks = exactRecord(record.routine_tasks, ["completed", "without_llm"]);
	return {
		capture: providerReadiness(record.capture),
		asr: providerReadiness(record.asr),
		router: providerReadiness(record.router),
		llm: providerReadiness(record.llm),
		speech: providerReadiness(record.speech),
		routine_tasks: {
			completed: decimal(tasks.completed),
			without_llm: decimal(tasks.without_llm),
		},
		product_ready: boolean(record.product_ready),
	};
}

function metadata(value: unknown): VoiceSnapshotMetadata {
	const record = exactRecord(value, [
		"session_id", "phase", "preferences", "resolved_language", "readiness", "voice_catalog",
		"cancel_available", "cleanup", "route_kind", "capture_elapsed_ms", "output_progress",
		"next_transcript_sequence", "next_answer_sequence", "notices", "error",
	]);
	const catalog = exactRecord(record.voice_catalog, ["revision"]);
	const progress = exactRecord(record.output_progress, ["queued_segments", "completed_segments", "active"]);
	const elapsed = nullable(record.capture_elapsed_ms, (item) => integer(item, 30_000));
	return {
		session_id: text(record.session_id, 128, false),
		phase: oneOf(record.phase, PHASES) as VoicePhase,
		preferences: preferences(record.preferences),
		resolved_language: nullable(record.resolved_language, language),
		readiness: readiness(record.readiness),
		voice_catalog: { revision: decimal(catalog.revision) },
		cancel_available: boolean(record.cancel_available),
		cleanup: cleanup(record.cleanup),
		route_kind: nullable(record.route_kind, (item) => oneOf(item, ROUTES) as RouteKind),
		capture_elapsed_ms: elapsed,
		output_progress: {
			queued_segments: integer(progress.queued_segments, 2),
			completed_segments: integer(progress.completed_segments, 255),
			active: boolean(progress.active),
		},
		next_transcript_sequence: decimal(record.next_transcript_sequence),
		next_answer_sequence: decimal(record.next_answer_sequence),
		notices: list(record.notices, 64, (item) => oneOf(item, NOTICES) as VoiceNoticeCode),
		error: nullable(record.error, (item) => oneOf(item, ERRORS) as VoiceErrorCode),
	};
}

function messageRole(value: unknown): MessageRole {
	return oneOf(value, ROLES);
}

function source(value: unknown): UtteranceSource {
	return oneOf(value, SOURCES);
}

function stream(value: unknown): PendingStream {
	return oneOf(value, STREAMS);
}

function inputDevice(value: unknown): VoiceInputDevice {
	const record = exactRecord(value, ["handle", "label", "is_default"]);
	return {
		handle: text(record.handle, 8_192, false),
		label: text(record.label, 8_192, false),
		is_default: boolean(record.is_default),
	};
}

function voiceRecord(value: unknown): VoiceCatalogRecord {
	const record = exactRecord(value, ["id", "display_name", "locale", "language"]);
	return {
		id: text(record.id, 8_192, false),
		display_name: text(record.display_name, 8_192, false),
		locale: text(record.locale, 8_192, false),
		language: language(record.language),
	};
}

interface SnapshotHeader {
	protocol_version: 1;
	event_sequence: DecimalU64;
	resync_id: DecimalU64;
	revision: DecimalU64;
	owner: VoiceOwner | null;
	page_count: number;
	total_text_bytes: number;
	total_metadata_bytes: number;
	digest_sha256: string;
}

interface BeginFrame extends SnapshotHeader {
	frame: "begin";
	metadata: VoiceSnapshotMetadata;
}

interface SnapshotPageFrame {
	frame: "page";
	protocol_version: 1;
	event_sequence: DecimalU64;
	resync_id: DecimalU64;
	revision: DecimalU64;
	owner: VoiceOwner | null;
	index: number;
	section: "history" | "pending" | "input_devices" | "voices";
	items: unknown[];
}

type ParsedSnapshotItem =
	| {
			kind: "history_message";
			message_index: number;
			id: string;
			role: MessageRole;
			language: VoiceLanguage;
			source: UtteranceSource;
			total_bytes: number;
			chunk_count: number;
	  }
	| {
			kind: "history_chunk";
			message_index: number;
			byte_offset: number;
			total_bytes: number;
			chunk_index: number;
			chunk_count: number;
			text: string;
	  }
	| {
			kind: "pending_chunk";
			stream: PendingStream;
			sequence: DecimalU64;
			language: VoiceLanguage;
			source: UtteranceSource;
			byte_offset: number;
			total_bytes: number;
			chunk_index: number;
			chunk_count: number;
			final_chunk: boolean;
			partial: boolean;
			text: string;
	  }
	| { kind: "input_device"; record: VoiceInputDevice }
	| { kind: "voice"; record: VoiceCatalogRecord };

interface EndFrame extends SnapshotHeader {
	frame: "end";
}

interface MessageHeader {
	id: string;
	role: MessageRole;
	language: VoiceLanguage;
	source: UtteranceSource;
	total_bytes: number;
	chunk_count: number;
	chunks: Map<number, { offset: number; text: string; bytes: number }>;
}

interface PendingHeader {
	stream: PendingStream;
	sequence: DecimalU64;
	language: VoiceLanguage;
	source: UtteranceSource;
	total_bytes: number;
	chunk_count: number;
	final_chunk: boolean;
	partial: boolean;
	chunks: Map<number, { offset: number; text: string; bytes: number }>;
}

function frameBytes(value: unknown): number {
	let serialized: string;
	try {
		serialized = JSON.stringify(value);
	} catch {
		return fail();
	}
	if (typeof serialized !== "string") return fail();
	const bytes = encoder.encode(serialized).byteLength;
	if (bytes > MAX_FRAME_BYTES) return fail();
	return bytes;
}

function digestText(value: unknown): string {
	const result = text(value, 64, false);
	if (!/^[0-9a-f]{64}$/u.test(result)) return fail();
	return result;
}

function parseHeader(record: Record<string, unknown>): SnapshotHeader {
	return {
		protocol_version: record.protocol_version === 1 ? 1 : fail(),
		event_sequence: decimal(record.event_sequence),
		resync_id: decimal(record.resync_id),
		revision: decimal(record.revision),
		owner: owner(record.owner),
		page_count: integer(record.page_count, MAX_PAGES),
		total_text_bytes: integer(record.total_text_bytes, MAX_SNAPSHOT_TEXT_BYTES),
		total_metadata_bytes: integer(record.total_metadata_bytes, MAX_SNAPSHOT_METADATA_BYTES),
		digest_sha256: digestText(record.digest_sha256),
	};
}

function parseBegin(value: unknown): BeginFrame {
	frameBytes(value);
	const record = exactRecord(value, [
		"frame", "protocol_version", "event_sequence", "resync_id", "revision", "owner",
		"page_count", "total_text_bytes", "total_metadata_bytes", "digest_sha256", "metadata",
	]);
	if (record.frame !== "begin") return fail();
	return { ...parseHeader(record), frame: "begin", metadata: metadata(record.metadata) };
}

function parsePage(value: unknown): SnapshotPageFrame {
	frameBytes(value);
	const record = exactRecord(value, [
		"frame", "protocol_version", "event_sequence", "resync_id", "revision", "owner",
		"index", "section", "items",
	]);
	if (record.frame !== "page" || record.protocol_version !== 1) return fail();
	if (!Array.isArray(record.items) || record.items.length > 8_192) return fail();
	return {
		frame: "page",
		protocol_version: 1,
		event_sequence: decimal(record.event_sequence),
		resync_id: decimal(record.resync_id),
		revision: decimal(record.revision),
		owner: owner(record.owner),
		index: integer(record.index, MAX_PAGES - 1),
		section: oneOf(record.section, ["history", "pending", "input_devices", "voices"] as const),
		items: record.items,
	};
}

function parseEnd(value: unknown): EndFrame {
	frameBytes(value);
	const record = exactRecord(value, [
		"frame", "protocol_version", "event_sequence", "resync_id", "revision", "owner",
		"page_count", "total_text_bytes", "total_metadata_bytes", "digest_sha256",
	]);
	if (record.frame !== "end") return fail();
	return { ...parseHeader(record), frame: "end" };
}

function headerMatches(begin: SnapshotHeader, frame: SnapshotHeader): boolean {
	return begin.protocol_version === frame.protocol_version &&
		begin.event_sequence === frame.event_sequence && begin.resync_id === frame.resync_id &&
		begin.revision === frame.revision && begin.page_count === frame.page_count &&
		begin.total_text_bytes === frame.total_text_bytes &&
		begin.total_metadata_bytes === frame.total_metadata_bytes &&
		begin.digest_sha256 === frame.digest_sha256 &&
		begin.owner?.session_id === frame.owner?.session_id && begin.owner?.turn_id === frame.owner?.turn_id &&
		begin.owner?.generation === frame.owner?.generation &&
		((begin.owner === null) === (frame.owner === null));
}

function parseSnapshotItem(value: unknown, page: SnapshotPageFrame): ParsedSnapshotItem {
	if (!isRecord(value)) return fail();
	switch (value.kind) {
		case "history_message": {
			const item = exactRecord(value, [
				"kind", "message_index", "id", "role", "language", "source", "total_bytes", "chunk_count",
			]);
			if (page.section !== "history") return fail();
			return {
				kind: "history_message",
				message_index: integer(item.message_index, MAX_HISTORY_MESSAGES - 1),
				id: text(item.id, 8_192, false),
				role: messageRole(item.role),
				language: language(item.language),
				source: source(item.source),
				total_bytes: integer(item.total_bytes, MAX_HISTORY_BYTES),
				chunk_count: integer(item.chunk_count, 128),
			};
		}
		case "history_chunk": {
			const item = exactRecord(value, [
				"kind", "message_index", "byte_offset", "total_bytes", "chunk_index", "chunk_count", "text",
			]);
			if (page.section !== "history") return fail();
			return {
				kind: "history_chunk",
				message_index: integer(item.message_index, MAX_HISTORY_MESSAGES - 1),
				byte_offset: integer(item.byte_offset, MAX_HISTORY_BYTES),
				total_bytes: integer(item.total_bytes, MAX_HISTORY_BYTES),
				chunk_index: integer(item.chunk_index, 128),
				chunk_count: integer(item.chunk_count, 128),
				text: text(item.text, MAX_PAGE_TEXT_BYTES),
			};
		}
		case "pending_chunk": {
			const item = exactRecord(value, [
				"kind", "stream", "sequence", "language", "source", "byte_offset", "total_bytes",
				"chunk_index", "chunk_count", "final_chunk", "partial", "text",
			]);
			if (page.section !== "pending") return fail();
			return {
				kind: "pending_chunk",
				stream: stream(item.stream),
				sequence: decimal(item.sequence),
				language: language(item.language),
				source: source(item.source),
				byte_offset: integer(item.byte_offset, MAX_SNAPSHOT_TEXT_BYTES),
				total_bytes: integer(item.total_bytes, MAX_SNAPSHOT_TEXT_BYTES),
				chunk_index: integer(item.chunk_index, 128),
				chunk_count: integer(item.chunk_count, 128),
				final_chunk: boolean(item.final_chunk),
				partial: boolean(item.partial),
				text: text(item.text, MAX_PAGE_TEXT_BYTES),
			};
		}
		case "input_device": {
			const item = exactRecord(value, ["kind", "record"]);
			if (page.section !== "input_devices") return fail();
			return { kind: "input_device", record: inputDevice(item.record) };
		}
		case "voice": {
			const item = exactRecord(value, ["kind", "record"]);
			if (page.section !== "voices") return fail();
			return { kind: "voice", record: voiceRecord(item.record) };
		}
		default:
			return fail();
	}
}

function parseChunkBytes(value: string): number {
	return encoder.encode(value).byteLength;
}

function addChunk(
	map: Map<number, { offset: number; text: string; bytes: number }>,
	chunkIndex: number,
	chunkCount: number,
	byteOffset: number,
	totalBytes: number,
	value: string,
): void {
	if (chunkCount < 1 || chunkIndex >= chunkCount || byteOffset > totalBytes || map.has(chunkIndex)) fail();
	const bytes = parseChunkBytes(value);
	if (bytes > MAX_PAGE_TEXT_BYTES || byteOffset + bytes > totalBytes) fail();
	map.set(chunkIndex, { offset: byteOffset, text: value, bytes });
}

function combineChunks(
	map: Map<number, { offset: number; text: string; bytes: number }>,
	chunkCount: number,
	totalBytes: number,
): string {
	if (map.size !== chunkCount || chunkCount < 1) return fail();
	let offset = 0;
	let result = "";
	for (let index = 0; index < chunkCount; index += 1) {
		const chunk = map.get(index);
		if (!chunk || chunk.offset !== offset) return fail();
		offset += chunk.bytes;
		result += chunk.text;
	}
	if (offset !== totalBytes) return fail();
	return result;
}

function canonicalDigestMaterial(snapshot: Omit<VoiceSnapshot, "revision" | "event_sequence">): string {
	const metadataValue = snapshot.metadata;
	const history = snapshot.history.map((message) => ({ ...message, text: "" }));
	const pending = snapshot.pending.map((item) => ({ ...item, text: "" }));
	return JSON.stringify({
		version: "saturday.voice.snapshot.digest.v1",
		metadata: metadataValue,
		owner: snapshot.owner,
		history,
		pending,
		input_devices: snapshot.input_devices,
		voices: snapshot.voices,
	});
}

async function sha256Hex(value: string): Promise<string> {
	const digest = new Uint8Array(await globalThis.crypto.subtle.digest("SHA-256", encoder.encode(value)));
	return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export async function assembleVoiceSnapshot(
	beginWire: unknown,
	fetchNext: (resyncId: DecimalU64, cursor: DecimalU64) => Promise<unknown>,
): Promise<VoiceSnapshot> {
	const begin = parseBegin(beginWire);
	const metadataValue = begin.metadata;
	const historyHeaders = new Map<number, MessageHeader>();
	const pendingHeaders = new Map<string, PendingHeader>();
	const devices: VoiceInputDevice[] = [];
	const voices: VoiceCatalogRecord[] = [];
	const deviceHandles = new Set<string>();
	const voiceIds = new Set<string>();
	let totalPageTextBytes = 0;
	let totalHistoryBytes = 0;

	for (let cursor = 0; cursor <= begin.page_count; cursor += 1) {
		const raw = await fetchNext(begin.resync_id, String(cursor));
		if (cursor === begin.page_count) {
			const end = parseEnd(raw);
			if (!headerMatches(begin, end)) return fail();
			break;
		}
		const page = parsePage(raw);
		if (
			page.index !== cursor || page.event_sequence !== begin.event_sequence ||
			page.resync_id !== begin.resync_id || page.revision !== begin.revision ||
			page.owner?.session_id !== begin.owner?.session_id || page.owner?.turn_id !== begin.owner?.turn_id ||
			page.owner?.generation !== begin.owner?.generation || ((page.owner === null) !== (begin.owner === null))
		)
			return fail();
		let pageTextBytes = 0;
		const pageMetadataItems: unknown[] = [];
		for (const rawItem of page.items) {
			const item = parseSnapshotItem(rawItem, page);
			switch (item.kind) {
				case "history_message": {
					if (historyHeaders.has(item.message_index)) return fail();
					const header: MessageHeader = { ...item, chunks: new Map() };
					if (header.chunk_count < 1) return fail();
					historyHeaders.set(item.message_index, header);
					totalHistoryBytes += header.total_bytes;
					pageMetadataItems.push({
						kind: "history_message", message_index: item.message_index, id: header.id,
						role: header.role, language: header.language, source: header.source,
						total_bytes: header.total_bytes, chunk_count: header.chunk_count,
					});
					break;
				}
				case "history_chunk": {
					const header = historyHeaders.get(item.message_index);
					if (!header || item.total_bytes !== header.total_bytes || item.chunk_count !== header.chunk_count) return fail();
					const bytes = parseChunkBytes(item.text);
					pageTextBytes += bytes;
					addChunk(header.chunks, item.chunk_index, header.chunk_count,
						item.byte_offset, header.total_bytes, item.text);
					pageMetadataItems.push({
						kind: "history_chunk", message_index: item.message_index,
						byte_offset: item.byte_offset, total_bytes: header.total_bytes,
						chunk_index: item.chunk_index, chunk_count: header.chunk_count, text: "",
					});
					break;
				}
				case "pending_chunk": {
					const key = `${item.stream}:${item.sequence}`;
					let header = pendingHeaders.get(key);
					if (!header) {
						header = {
							stream: item.stream, sequence: item.sequence, language: item.language,
							source: item.source, total_bytes: item.total_bytes,
							chunk_count: item.chunk_count, final_chunk: item.final_chunk,
							partial: item.partial, chunks: new Map(),
						};
						if (header.chunk_count < 1) return fail();
						pendingHeaders.set(key, header);
					} else if (
						header.total_bytes !== item.total_bytes || header.chunk_count !== item.chunk_count ||
						header.language !== item.language || header.source !== item.source ||
						header.final_chunk !== item.final_chunk || header.partial !== item.partial
					) return fail();
					const bytes = parseChunkBytes(item.text);
					pageTextBytes += bytes;
					addChunk(header.chunks, item.chunk_index, header.chunk_count,
						item.byte_offset, header.total_bytes, item.text);
					pageMetadataItems.push({
						kind: "pending_chunk", stream: header.stream, sequence: header.sequence,
						language: header.language, source: header.source,
						byte_offset: item.byte_offset, total_bytes: header.total_bytes,
						chunk_index: item.chunk_index, chunk_count: header.chunk_count,
						final_chunk: header.final_chunk, partial: header.partial, text: "",
					});
					break;
				}
				case "input_device": {
					const device = item.record;
					if (deviceHandles.has(device.handle) || devices.length >= MAX_DEVICES) return fail();
					deviceHandles.add(device.handle);
					devices.push(device);
					pageMetadataItems.push({ kind: "input_device", record: device });
					break;
				}
				case "voice": {
					const voice = item.record;
					if (voiceIds.has(voice.id) || voices.length >= MAX_VOICES) return fail();
					voiceIds.add(voice.id);
					voices.push(voice);
					pageMetadataItems.push({ kind: "voice", record: voice });
					break;
				}
			}
		}
		if (pageTextBytes > MAX_PAGE_TEXT_BYTES) return fail();
		totalPageTextBytes += pageTextBytes;
		const pageMetadataBytes = encoder.encode(JSON.stringify({
			frame: "page", protocol_version: 1, event_sequence: page.event_sequence,
			resync_id: page.resync_id, revision: page.revision, owner: page.owner,
			index: page.index, section: page.section, items: pageMetadataItems,
		})).byteLength;
		if (pageMetadataBytes > MAX_PAGE_METADATA_BYTES) return fail();
	}

	if (
		historyHeaders.size > MAX_HISTORY_MESSAGES || totalHistoryBytes > MAX_HISTORY_BYTES ||
		pendingHeaders.size > 2 || totalPageTextBytes !== begin.total_text_bytes
	)
		return fail();
	const history: VoiceHistoryMessage[] = [];
	for (let index = 0; index < historyHeaders.size; index += 1) {
		const header = historyHeaders.get(index);
		if (!header) return fail();
		history.push({
			id: header.id, role: header.role, language: header.language, source: header.source,
			text: combineChunks(header.chunks, header.chunk_count, header.total_bytes),
		});
	}
	const pending: VoicePendingText[] = [];
	for (const [key, header] of pendingHeaders) {
		if (key !== `${header.stream}:${header.sequence}`) return fail();
		pending.push({
			stream: header.stream, sequence: header.sequence, language: header.language,
			source: header.source,
			text: combineChunks(header.chunks, header.chunk_count, header.total_bytes),
			final_chunk: header.final_chunk, partial: header.partial,
		});
	}
	const snapshotWithoutCursors = {
		metadata: metadataValue,
		owner: begin.owner,
		history,
		pending,
		input_devices: devices,
		voices,
	};
	const metadataBytes = encoder.encode(canonicalDigestMaterial(snapshotWithoutCursors)).byteLength;
	if (metadataBytes !== begin.total_metadata_bytes || metadataBytes > MAX_SNAPSHOT_METADATA_BYTES) return fail();
	if (await sha256Hex(canonicalDigestMaterial(snapshotWithoutCursors)) !== begin.digest_sha256) return fail();
	return {
		revision: begin.revision,
		event_sequence: begin.event_sequence,
		...snapshotWithoutCursors,
	};
}

function parseCaptureLifecycle(value: unknown): VoiceCaptureLifecycleState {
	return oneOf(value, ["permission_pending", "started", "permission_denied", "device_lost", "cancelled", "failed"] as const);
}

function parseIntent(value: unknown): AppIntentDescriptor {
	if (!isRecord(value) || typeof value.intent !== "string") return fail();
	switch (value.intent) {
		case "overview":
		case "conversation": {
			const record = exactRecord(value, ["intent", "owner", "intent_id"]);
			const parsedOwner = owner(record.owner);
			if (parsedOwner === null) return fail();
			if (value.intent === "overview")
				return { intent: "overview", owner: parsedOwner, intent_id: decimal(record.intent_id) };
			return { intent: "conversation", owner: parsedOwner, intent_id: decimal(record.intent_id) };
		}
		case "motion": {
			const record = exactRecord(value, ["intent", "owner", "intent_id", "enabled"]);
			const parsedOwner = owner(record.owner);
			if (parsedOwner === null) return fail();
			return {
				intent: "motion", owner: parsedOwner,
				intent_id: decimal(record.intent_id), enabled: boolean(record.enabled),
			};
		}
		default:
			return fail();
	}
}

function parseEvent(value: unknown): VoiceEventWire {
	const record = exactRecord(value, ["protocol_version", "event_sequence", "revision", "owner", "event"]);
	if (record.protocol_version !== 1 || !isRecord(record.event)) return fail();
	const eventRecord = record.event;
	let event: VoiceEventKind;
	switch (eventRecord.type) {
		case "state_changed": {
			const item = exactRecord(eventRecord, ["type", "phase", "route_kind", "cancel_available", "cleanup"]);
			event = {
				type: "state_changed", phase: oneOf(item.phase, PHASES) as VoicePhase,
				route_kind: nullable(item.route_kind, (field) => oneOf(field, ROUTES) as RouteKind),
				cancel_available: boolean(item.cancel_available), cleanup: cleanup(item.cleanup),
			};
			break;
		}
		case "capture_lifecycle": {
			const item = exactRecord(eventRecord, ["type", "state"]);
			event = { type: "capture_lifecycle", state: parseCaptureLifecycle(item.state) };
			break;
		}
		case "transcript_delta": {
			const item = exactRecord(eventRecord, ["type", "sequence", "text", "final_chunk", "language", "source"]);
			event = {
				type: "transcript_delta", sequence: decimal(item.sequence),
				text: text(item.text, MAX_TEXT_DELTA_BYTES), final_chunk: boolean(item.final_chunk),
				language: nullable(item.language, language), source: oneOf(item.source, SOURCES),
			};
			break;
		}
		case "answer_delta": {
			const item = exactRecord(eventRecord, ["type", "sequence", "text", "final_chunk", "language", "partial"]);
			event = {
				type: "answer_delta", sequence: decimal(item.sequence),
				text: text(item.text, MAX_TEXT_DELTA_BYTES), final_chunk: boolean(item.final_chunk),
				language: language(item.language), partial: boolean(item.partial),
			};
			break;
		}
		case "capture_meter": {
			const item = exactRecord(eventRecord, ["type", "elapsed_ms", "level"]);
			if (typeof item.level !== "number" || !Number.isFinite(item.level) || item.level < 0 || item.level > 1) return fail();
			event = { type: "capture_meter", elapsed_ms: integer(item.elapsed_ms, 30_000), level: item.level };
			break;
		}
		case "intent": {
			const item = exactRecord(eventRecord, ["type", "descriptor"]);
			event = { type: "intent", descriptor: parseIntent(item.descriptor) };
			break;
		}
		case "notice": {
			const item = exactRecord(eventRecord, ["type", "code"]);
			event = { type: "notice", code: oneOf(item.code, NOTICES) as VoiceNoticeCode };
			break;
		}
		case "failure": {
			const item = exactRecord(eventRecord, ["type", "code"]);
			event = { type: "failure", code: oneOf(item.code, ERRORS) as VoiceErrorCode };
			break;
		}
		case "output_started":
			exactRecord(eventRecord, ["type"]);
			event = { type: "output_started" };
			break;
		case "output_drained":
			exactRecord(eventRecord, ["type"]);
			event = { type: "output_drained" };
			break;
		case "terminal": {
			const item = exactRecord(eventRecord, ["type", "receipt"]);
			event = { type: "terminal", receipt: cleanup(item.receipt) };
			break;
		}
		default:
			return fail();
	}
	return {
		protocol_version: 1,
		event_sequence: decimal(record.event_sequence),
		revision: decimal(record.revision),
		owner: owner(record.owner),
		event,
	};
}

export interface ParsedEventBatch {
	protocol_version: 1;
	after_event_sequence: DecimalU64;
	latest_event_sequence: DecimalU64;
	resync_required: boolean;
	events: VoiceEventWire[];
}

export function parseVoiceEventBatch(value: unknown, expectedCursor: DecimalU64): ParsedEventBatch {
	frameBytes(value);
	const record = exactRecord(value, [
		"protocol_version", "after_event_sequence", "latest_event_sequence", "resync_required", "events",
	]);
	const after = decimal(record.after_event_sequence);
	const latest = decimal(record.latest_event_sequence);
	const resyncRequired = boolean(record.resync_required);
	const events = list(record.events, MAX_EVENT_BATCH, parseEvent);
	if (record.protocol_version !== 1 || after !== expectedCursor) return fail();
	if (compareDecimalU64(latest, after) < 0 || (resyncRequired && events.length !== 0)) return fail();
	if (!resyncRequired) {
		let cursor = after;
		for (const event of events) {
			if (sequenceRelation(cursor, event.event_sequence) !== "next") return fail();
			cursor = event.event_sequence;
		}
		if (compareDecimalU64(cursor, latest) > 0) return fail();
		if (events.length === 0 && latest !== after) return fail();
	}
	return {
		protocol_version: 1,
		after_event_sequence: after,
		latest_event_sequence: latest,
		resync_required: resyncRequired,
		events,
	};
}

function parseTransfer(value: unknown): {
	resync_id: DecimalU64;
	event_sequence: DecimalU64;
	revision: DecimalU64;
	owner: VoiceOwner | null;
} | null {
	if (value === null) return null;
	const record = exactRecord(value, ["resync_id", "event_sequence", "revision", "owner"]);
	return {
		resync_id: decimal(record.resync_id),
		event_sequence: decimal(record.event_sequence),
		revision: decimal(record.revision),
		owner: owner(record.owner),
	};
}

export interface VoiceCommandResult {
	accepted: boolean;
	code: "accepted" | "pending" | "completed" | "no_change";
	transfer: {
		resync_id: DecimalU64;
		event_sequence: DecimalU64;
		revision: DecimalU64;
		owner: VoiceOwner | null;
	} | null;
}

export function parseVoiceCommandResult(value: unknown): VoiceCommandResult {
	frameBytes(value);
	const record = exactRecord(value, ["accepted", "code", "transfer"]);
	return {
		accepted: boolean(record.accepted),
		code: oneOf(record.code, ["accepted", "pending", "completed", "no_change"] as const),
		transfer: parseTransfer(record.transfer),
	};
}

const ASSET_PURPOSES = ["runtime", "asr_weights", "llm_weights", "portable_voice"] as const;
const ASSET_RETENTION = [
	"app_managed_until_explicit_removal",
	"build_inputs_retained_for_provenance",
	"removed_after_promotion",
] as const;
const ASSET_STATES = ["blocked", "awaiting_approval", "ready", "disabled", "recovery_required"] as const;
const ASSET_CODES = [
	"not_reviewed",
	"source_review_required",
	"runtime_build_required",
	"model_approval_required",
	"owner_trust_setup_required",
	"transfer_not_granted",
	"asset_missing",
	"verified",
	"disabled",
	"recovery_required",
] as const;
const DIGEST_STATES = ["not_published", "observed_unreviewed", "independently_verified"] as const;

function sourceInput(value: unknown): AssetSourceInputDisclosure {
	const record = exactRecord(value, [
		"name",
		"origin",
		"revision",
		"source_tree_id",
		"maximum_transfer_bytes",
		"observed_bytes",
		"archive_sha256",
		"digest_status",
		"license_ids",
		"notice_ids",
	]);
	return {
		name: text(record.name, 8_192, false),
		origin: text(record.origin, 8_192, false),
		revision: text(record.revision, 8_192, false),
		source_tree_id: text(record.source_tree_id, 8_192, false),
		maximum_transfer_bytes: decimal(record.maximum_transfer_bytes),
		observed_bytes: nullable(record.observed_bytes, decimal),
		archive_sha256: nullable(record.archive_sha256, digestText),
		digest_status: oneOf(record.digest_status, DIGEST_STATES) as AssetDigestStatus,
		license_ids: list(record.license_ids, MAX_ASSET_LIST_ITEMS, (item) => text(item, 8_192, false)),
		notice_ids: list(record.notice_ids, MAX_ASSET_LIST_ITEMS, (item) => text(item, 8_192, false)),
	};
}

function assetDisclosure(value: unknown): AssetDisclosure {
	const record = exactRecord(value, [
		"candidate_name",
		"version",
		"source_origin",
		"source_revision",
		"purpose",
		"expected_bytes",
		"digest_sha256",
		"license_ids",
		"notice_ids",
		"max_staging_bytes",
		"max_retained_bytes",
		"max_disk_bytes",
		"redirect_origins",
		"retention",
		"build",
	]);
	let build: AssetBuildDisclosure | null = null;
	if (record.build !== null) {
		const candidate = exactRecord(record.build, [
			"source_inputs",
			"toolchain_identity",
			"recipe_identity",
			"maximum_workspace_bytes",
			"maximum_output_bytes",
			"cpu_limit",
			"memory_limit_bytes",
			"deadline_seconds",
			"network_policy",
			"write_policy",
		]);
		build = {
			source_inputs: list(candidate.source_inputs, MAX_ASSET_LIST_ITEMS, sourceInput),
			toolchain_identity: text(candidate.toolchain_identity, 8_192, false),
			recipe_identity: text(candidate.recipe_identity, 8_192, false),
			maximum_workspace_bytes: decimal(candidate.maximum_workspace_bytes),
			maximum_output_bytes: decimal(candidate.maximum_output_bytes),
			cpu_limit: integer(candidate.cpu_limit, 256),
			memory_limit_bytes: decimal(candidate.memory_limit_bytes),
			deadline_seconds: integer(candidate.deadline_seconds, 86_400),
			network_policy: oneOf(candidate.network_policy, ["none"] as const),
			write_policy: oneOf(candidate.write_policy, ["app_owned_build_workspace_only"] as const),
		};
	}
	return {
		candidate_name: text(record.candidate_name, 8_192, false),
		version: text(record.version, 8_192, false),
		source_origin: text(record.source_origin, 8_192, false),
		source_revision: text(record.source_revision, 8_192, false),
		purpose: oneOf(record.purpose, ASSET_PURPOSES) as AssetPurpose,
		expected_bytes: decimal(record.expected_bytes),
		digest_sha256: digestText(record.digest_sha256),
		license_ids: list(record.license_ids, MAX_ASSET_LIST_ITEMS, (item) => text(item, 8_192, false)),
		notice_ids: list(record.notice_ids, MAX_ASSET_LIST_ITEMS, (item) => text(item, 8_192, false)),
		max_staging_bytes: decimal(record.max_staging_bytes),
		max_retained_bytes: decimal(record.max_retained_bytes),
		max_disk_bytes: decimal(record.max_disk_bytes),
		redirect_origins: list(record.redirect_origins, MAX_ASSET_LIST_ITEMS, (item) => text(item, 8_192, false)),
		retention: oneOf(record.retention, ASSET_RETENTION) as AssetRetention,
		build,
	};
}

function assetStatus(value: unknown): AssetStatus {
	const record = exactRecord(value, ["asset_id", "purpose", "status", "code", "asset_disclosure"]);
	const purpose = oneOf(record.purpose, ASSET_PURPOSES) as AssetPurpose;
	const disclosure = assetDisclosure(record.asset_disclosure);
	if (purpose !== disclosure.purpose) return fail();
	return {
		asset_id: text(record.asset_id, 8_192, false),
		purpose,
		status: oneOf(record.status, ASSET_STATES) as AssetState,
		code: oneOf(record.code, ASSET_CODES) as AssetStatusCode,
		asset_disclosure: disclosure,
	};
}

export function parseAssetCatalog(value: unknown): AssetCatalog {
	frameBytes(value);
	const record = exactRecord(value, ["revision", "assets"]);
	const assets = list(record.assets, MAX_ASSET_LIST_ITEMS, assetStatus);
	const ids = new Set<string>();
	for (const asset of assets) {
		if (ids.has(asset.asset_id)) return fail();
		ids.add(asset.asset_id);
	}
	return { revision: decimal(record.revision), assets };
}

export function parseAssetInstallOutcome(value: unknown): AssetInstallOutcome {
	frameBytes(value);
	const record = exactRecord(value, ["accepted", "status"]);
	return { accepted: boolean(record.accepted), status: assetStatus(record.status) };
}

export function parseAssetRemovalOutcome(value: unknown): AssetRemovalOutcome {
	frameBytes(value);
	const record = exactRecord(value, ["removed", "catalog"]);
	return { removed: boolean(record.removed), catalog: parseAssetCatalog(record.catalog) };
}

export function assertRequestEnvelope(value: unknown): void {
	const record = exactRecord(value, ["request"]);
	if (!isRecord(record.request)) fail();
	if (Object.hasOwn(record.request, "command")) fail();
}
