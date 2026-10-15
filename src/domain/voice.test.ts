import { describe, expect, it } from "vitest";
import {
	compareDecimalU64,
	isDecimalU64,
	reduceVoiceEvent,
	sequenceRelation,
	validateReviewedTranscript,
	validateTypedText,
	type VoiceEventWire,
	type VoiceOwner,
	type VoicePhase,
	type VoiceReducerState,
	type VoiceSnapshot,
} from "./voice";

describe("canonical native counters", () => {
	it("accepts only canonical in-range decimal u64 strings", () => {
		for (const value of ["0", "1", "9007199254740993", "18446744073709551615"])
			expect(isDecimalU64(value)).toBe(true);
		for (const value of [
			0,
			1n,
			"",
			"00",
			"01",
			"+1",
			"-1",
			"1.0",
			"18446744073709551616",
		])
			expect(isDecimalU64(value)).toBe(false);
	});

	it("orders values above Number.MAX_SAFE_INTEGER without precision loss", () => {
		expect(compareDecimalU64("9007199254740993", "9007199254740992")).toBe(1);
		expect(compareDecimalU64("9007199254740992", "9007199254740993")).toBe(-1);
		expect(compareDecimalU64("18446744073709551615", "18446744073709551615")).toBe(0);
	});

	it("distinguishes forward, duplicate, stale and missing event sequences", () => {
		expect(sequenceRelation("9007199254740993", "9007199254740994")).toBe("next");
		expect(sequenceRelation("9007199254740993", "9007199254740993")).toBe("duplicate");
		expect(sequenceRelation("9007199254740994", "9007199254740993")).toBe("stale");
		expect(sequenceRelation("9007199254740993", "9007199254740995")).toBe("gap");
	});
});

describe("voice text limits", () => {
	it("accepts typed text at both scalar and UTF-8 byte limits", () => {
		expect(validateTypedText("😀".repeat(500))).toMatchObject({
			ok: true,
			scalar_count: 500,
			utf8_bytes: 2000,
		});
		expect(validateTypedText("é".repeat(500))).toMatchObject({
			ok: true,
			scalar_count: 500,
			utf8_bytes: 1000,
		});
	});

	it("rejects typed overflow instead of slicing or submitting it", () => {
		expect(validateTypedText("x".repeat(501))).toMatchObject({
			ok: false,
			reason: "too_many_scalars",
		});
		expect(validateTypedText("😀".repeat(501))).toMatchObject({
			ok: false,
			reason: "too_many_scalars",
		});
	});

	it("uses spoken-review limits and rejects malformed UTF-16", () => {
		expect(validateReviewedTranscript("😀".repeat(2000))).toMatchObject({
			ok: true,
			scalar_count: 2000,
			utf8_bytes: 8000,
		});
		expect(validateReviewedTranscript("😀".repeat(2001))).toMatchObject({
			ok: false,
			reason: "too_many_scalars",
		});
		expect(validateTypedText("\uD800")).toMatchObject({
			ok: false,
			reason: "invalid_unicode",
		});
	});
});

const owner = {
	session_id: "session-a",
	turn_id: "turn-a",
	generation: "9007199254740993",
} as const;

function idleSnapshot(): VoiceSnapshot {
	return {
		revision: "9007199254740993",
		event_sequence: "9007199254740993",
		metadata: {
			session_id: owner.session_id,
			phase: "idle",
			preferences: {
				language_mode: "auto",
				input_device_handle: null,
				voice_ids: { tr: null, en: null },
				spoken_reply: false,
				review_before_send: false,
			},
			resolved_language: null,
			readiness: {
				capture: { state: "setup_required", code: "asset_approval_required" },
				asr: { state: "setup_required", code: "asset_approval_required" },
				router: { state: "ready", code: "ready" },
				llm: { state: "setup_required", code: "asset_approval_required" },
				speech: { state: "setup_required", code: "asset_approval_required" },
				routine_tasks: { completed: "0", without_llm: "0" },
				product_ready: false,
			},
			voice_catalog: { revision: "0" },
			cancel_available: false,
			cleanup: { stopped: true, verified: true, code: "no_resources_owned" },
			route_kind: null,
			capture_elapsed_ms: null,
			output_progress: { queued_segments: 0, completed_segments: 0, active: false },
			next_transcript_sequence: "9007199254740993",
			next_answer_sequence: "0",
			notices: [],
			error: null,
		},
		owner,
		history: [],
		pending: [],
		input_devices: [],
		voices: [],
	};
}

function reducerState(snapshot = idleSnapshot()): VoiceReducerState {
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

function stateEvent(
	event_sequence: string,
	eventOwner: VoiceOwner,
	phase: VoicePhase,
): VoiceEventWire {
	return {
		protocol_version: 1,
		event_sequence,
		revision: event_sequence,
		owner: eventOwner,
		event: {
			type: "state_changed",
			phase,
			route_kind: null,
			cancel_available: phase === "listening",
			cleanup:
				phase === "cancelling"
					? { stopped: false, verified: false, code: "failed" }
					: { stopped: true, verified: true, code: "no_resources_owned" },
		},
	};
}

describe("voice event reducer", () => {
	it("applies native state only for the exact next global u64 event", () => {
		const initial = reducerState();
		const result = reduceVoiceEvent(
			initial,
			stateEvent("9007199254740994", owner, "listening"),
		);

		expect(result.outcome).toBe("applied");
		expect(result.state.event_sequence).toBe("9007199254740994");
		expect(result.state.snapshot.metadata.phase).toBe("listening");
	});

	it("accepts the next transcript stream sequence above the safe integer range", () => {
		const initial = reducerState();
		const delta: VoiceEventWire = {
			protocol_version: 1,
			event_sequence: "9007199254740994",
			revision: "9007199254740994",
			owner,
			event: {
				type: "transcript_delta",
				sequence: "9007199254740993",
				text: "Merhaba",
				final_chunk: true,
				language: "tr",
				source: "speech",
			},
		};

		const result = reduceVoiceEvent(initial, delta);

		expect(result.outcome).toBe("applied");
		expect(result.state.snapshot.metadata.next_transcript_sequence).toBe(
			"9007199254740994",
		);
		expect(result.state.transcript_deltas[0]).toMatchObject({
			sequence: "9007199254740993",
			text: "Merhaba",
			language: "tr",
			final_chunk: true,
		});
	});

	it("drops stale-owner state without changing the active session", () => {
		const initial = reducerState();
		const result = reduceVoiceEvent(
			initial,
			stateEvent(
				"9007199254740994",
				{ session_id: "old", turn_id: "old", generation: "4" },
				"speaking",
			),
		);

		expect(result.outcome).toBe("ignored");
		expect(result.state.snapshot.metadata.phase).toBe("idle");
		expect(result.state.event_sequence).toBe("9007199254740994");
	});

	it("requires a new snapshot after a global event gap", () => {
		const initial = reducerState();
		const result = reduceVoiceEvent(
			initial,
			stateEvent("9007199254740995", owner, "speaking"),
		);

		expect(result.outcome).toBe("resync");
		expect(result.state).toBe(initial);
	});

	it("forces resynchronization when a per-stream transcript delta is skipped", () => {
		const initial = reducerState();
		const delta: VoiceEventWire = {
			protocol_version: 1,
			event_sequence: "9007199254740994",
			revision: "9007199254740994",
			owner,
			event: {
				type: "transcript_delta",
				sequence: "9007199254740994",
				text: "Hello",
				final_chunk: false,
				language: "en",
				source: "speech",
			},
		};

		const result = reduceVoiceEvent(initial, delta);

		expect(result.outcome).toBe("resync");
		expect(result.state).toBe(initial);
	});
});
