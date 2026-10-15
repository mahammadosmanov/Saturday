import { describe, expect, it } from "vitest";
import type { DecimalU64, VoiceSnapshotMetadata } from "../domain/voice";
import {
	createVoiceClient,
	type VoiceNativeTransport,
	type VoiceSubscription,
} from "./voice-client";

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
function deferred<T>() {
	let resolve!: (value: T | PromiseLike<T>) => void;
	let reject!: (reason?: unknown) => void;
	const promise = new Promise<T>((resolvePromise, rejectPromise) => {
		resolve = resolvePromise;
		reject = rejectPromise;
	});
	return { promise, resolve, reject };
}

function requestFrom(rawRequest: Uint8Array): Record<string, unknown> {
	const decoded: unknown = JSON.parse(decoder.decode(rawRequest));
	if (
		typeof decoded !== "object" ||
		decoded === null ||
		!("request" in decoded) ||
		typeof decoded.request !== "object" ||
		decoded.request === null
	)
		throw new Error("Expected a voice request envelope encoded as raw UTF-8");
	return decoded.request as Record<string, unknown>;
}

function emptyEventBatch(rawRequest: Uint8Array, latest: DecimalU64) {
	const request = requestFrom(rawRequest);
	return {
		protocol_version: 1,
		after_event_sequence: request.after_event_sequence,
		latest_event_sequence: latest,
		resync_required: false,
		events: [],
	};
}

function metadata(): VoiceSnapshotMetadata {
	return {
		session_id: "test-session",
		phase: "idle",
		preferences: {
			language_mode: null,
			input_device_handle: null,
			voice_ids: { tr: null, en: null },
			spoken_reply: false,
			review_before_send: false,
		},
		resolved_language: null,
		readiness: {
			capture: { state: "unavailable", code: "not_inspected" },
			asr: { state: "unavailable", code: "not_inspected" },
			router: { state: "unavailable", code: "not_inspected" },
			llm: { state: "unavailable", code: "not_inspected" },
			speech: { state: "unavailable", code: "not_inspected" },
			routine_tasks: { completed: "0", without_llm: "0" },
			product_ready: false,
		},
		voice_catalog: { revision: "0" },
		cancel_available: false,
		cleanup: { stopped: true, verified: true, code: "no_resources_owned" },
		route_kind: null,
		capture_elapsed_ms: null,
		output_progress: { queued_segments: 0, completed_segments: 0, active: false },
		next_transcript_sequence: "0",
		next_answer_sequence: "0",
		notices: [],
		error: null,
	};
}

async function snapshotFrames(eventSequence: DecimalU64) {
	const snapshot = {
		metadata: metadata(),
		owner: null,
		history: [],
		pending: [],
		input_devices: [],
		voices: [],
	};
	const digestMaterial = {
		version: "saturday.voice.snapshot.digest.v1",
		metadata: snapshot.metadata,
		owner: snapshot.owner,
		history: snapshot.history,
		pending: snapshot.pending,
		input_devices: snapshot.input_devices,
		voices: snapshot.voices,
	};
	const serialized = JSON.stringify(digestMaterial);
	const digest = new Uint8Array(
		await crypto.subtle.digest("SHA-256", encoder.encode(serialized)),
	);
	const digestSha256 = Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
	const shared = {
		protocol_version: 1,
		event_sequence: eventSequence,
		resync_id: "9",
		revision: "11",
		owner: null,
		page_count: 0,
		total_text_bytes: 0,
		total_metadata_bytes: encoder.encode(serialized).byteLength,
		digest_sha256: digestSha256,
	};
	return {
		begin: { frame: "begin", ...shared, metadata: snapshot.metadata },
		end: { frame: "end", ...shared },
	};
}

class TestTransport implements VoiceNativeTransport {
	readonly calls: Array<{ command: string; rawRequest: Uint8Array }> = [];

	constructor(
		readonly native: boolean,
		private readonly handler: (command: string, rawRequest: Uint8Array) => Promise<unknown>,
	) {}

	isNative() {
		return this.native;
	}

	invoke(command: string, rawRequest: Uint8Array) {
		this.calls.push({ command, rawRequest });
		return this.handler(command, rawRequest);
	}
}

describe("native voice client", () => {
	it("never invokes a browser microphone or fallback when native voice is absent", async () => {
		const transport = new TestTransport(false, async () => {
			throw new Error("unexpected native invocation");
		});
		const client = createVoiceClient(transport);
		expect(client.isSupported()).toBe(false);
		await expect(client.submitText("hello", "en")).rejects.toThrow();
		expect(transport.calls).toEqual([]);
	});

	it("rejects typed overflow before a native command is invoked", async () => {
		const transport = new TestTransport(true, async () => ({ accepted: true, code: "accepted", transfer: null }));
		const client = createVoiceClient(transport);
		await expect(client.submitText("x".repeat(501), "en")).rejects.toThrow();
		expect(transport.calls).toEqual([]);
	});

	it("uses a raw UTF-8 envelope for typed native requests", async () => {
		const transport = new TestTransport(true, async () => ({ accepted: true, code: "accepted", transfer: null }));
		const client = createVoiceClient(transport);
		await client.submitText("hello", "en");
		expect(
			transport.calls.map(({ command, rawRequest }) => [command, decoder.decode(rawRequest)]),
		).toEqual([
			[
				"voice_submit_text",
				JSON.stringify({ request: { protocol_version: 1, text: "hello", language_mode: "en" } }),
			],
		]);
	});

	it("publishes only a verified snapshot and preserves high u64 cursors", async () => {
		const eventSequence = "9007199254740997";
		const frames = await snapshotFrames(eventSequence);
		const transport = new TestTransport(true, async (command, args) => {
			if (command !== "voice_get_status") throw new Error(`unexpected command ${command}`);
			const request = requestFrom(args);
			if (request.mode === "snapshot_begin") return frames.begin;
			if (request.mode === "snapshot_next") return frames.end;
			if (request.after_event_sequence === "0")
				return {
					protocol_version: 1,
					after_event_sequence: "0",
					latest_event_sequence: eventSequence,
					resync_required: true,
					events: [],
				};
			return emptyEventBatch(args, eventSequence);
		});
		const client = createVoiceClient(transport);
		const received = deferred<DecimalU64>();
		const subscription: VoiceSubscription = client.watch((update) => {
			if (update.type === "snapshot") received.resolve(update.snapshot.event_sequence);
		}, received.reject);
		expect(await received.promise).toBe(eventSequence);
		await subscription.close();
		expect(transport.calls[0]?.command).toBe("voice_get_status");
	});
	it("refuses cancellation for a stale owner without invoking native cancel", async () => {
		const frames = await snapshotFrames("0");
		const activeOwner = {
			session_id: "session-current",
			turn_id: "turn-current",
			generation: "18446744073709551615",
		};
		const staleOwner = { ...activeOwner, session_id: "session-stale" };
		let started = false;
		const transport = new TestTransport(true, async (command, rawRequest) => {
			const request = requestFrom(rawRequest);
			if (command === "voice_start") {
				started = true;
				return {
					accepted: true,
					code: "accepted",
					transfer: { resync_id: "0", event_sequence: "0", revision: "0", owner: activeOwner },
				};
			}
			if (command !== "voice_get_status") throw new Error(`unexpected command ${command}`);
			if (request.mode === "snapshot_begin") return frames.begin;
			if (request.mode === "snapshot_next") return frames.end;
			if (started && request.after_event_sequence === "0") {
				return {
					protocol_version: 1,
					after_event_sequence: "0",
					latest_event_sequence: "1",
					resync_required: false,
					events: [{
						protocol_version: 1,
						event_sequence: "1",
						revision: "12",
						owner: activeOwner,
						event: {
							type: "state_changed",
							phase: "listening",
							route_kind: null,
							cancel_available: true,
							cleanup: { stopped: false, verified: false, code: "no_resources_owned" },
						},
					}],
				};
			}
			return emptyEventBatch(rawRequest, started ? "1" : "0");
		});
		const client = createVoiceClient(transport);
		const baseline = deferred<void>();
		const turn = deferred<void>();
		const subscription = client.watch((update) => {
			if (update.type === "snapshot") baseline.resolve(undefined);
			if (update.type === "events" && update.state.snapshot.owner) turn.resolve(undefined);
		}, baseline.reject);
		await baseline.promise;
		await client.start("auto", null);
		await turn.promise;
		await expect(subscription.cancelAndWait(staleOwner, "user")).rejects.toThrow();
		expect(transport.calls.filter(({ command }) => command === "voice_cancel")).toHaveLength(0);
		await subscription.close();
	});

	it("rebuilds a verified snapshot after the native event watermark requires resync", async () => {
		const first = await snapshotFrames("0");
		const recovered = await snapshotFrames("5");
		let begins = 0;
		const transport = new TestTransport(true, async (command, rawRequest) => {
			if (command !== "voice_get_status") throw new Error(`unexpected command ${command}`);
			const request = requestFrom(rawRequest);
			if (request.mode === "snapshot_begin") {
				begins += 1;
				return begins === 1 ? first.begin : recovered.begin;
			}
			if (request.mode === "snapshot_next")
				return begins === 1 ? first.end : recovered.end;
			if (request.after_event_sequence === "0") {
				return {
					protocol_version: 1,
					after_event_sequence: "0",
					latest_event_sequence: "5",
					resync_required: true,
					events: [],
				};
			}
			return emptyEventBatch(rawRequest, "5");
		});
		const client = createVoiceClient(transport);
		const received = deferred<DecimalU64>();
		const subscription = client.watch((update) => {
			if (update.type === "snapshot" && update.snapshot.event_sequence === "5")
				received.resolve(update.snapshot.event_sequence);
		}, received.reject);
		expect(await received.promise).toBe("5");
		await subscription.close();
		expect(begins).toBeGreaterThanOrEqual(2);
	});

	it("rejects unknown snapshot fields rather than publishing a partial state", async () => {
		const transport = new TestTransport(true, async (command, args) => {
			if (command !== "voice_get_status") throw new Error(`unexpected command ${command}`);
			const request = requestFrom(args);
			if (request.mode === "events") return emptyEventBatch(args, "0");
			return {
				frame: "begin",
				protocol_version: 1,
				event_sequence: "0",
				resync_id: "0",
				revision: "0",
				owner: null,
				page_count: 0,
				total_text_bytes: 0,
				total_metadata_bytes: 0,
				digest_sha256: "0".repeat(64),
				metadata: metadata(),
				unexpected: true,
			};
		});
		const client = createVoiceClient(transport);
		const observed = deferred<Error>();
		const subscription = client.watch(
			() => observed.resolve(new Error("invalid wire was published")),
			observed.resolve,
		);
		expect(await observed.promise).toBeInstanceOf(Error);
		await subscription.close();
	});

	it("rejects snapshot pagination beyond the native page cap", async () => {
		const frames = await snapshotFrames("0");
		const transport = new TestTransport(true, async (command, args) => {
			if (command !== "voice_get_status") throw new Error(`unexpected command ${command}`);
			const request = requestFrom(args);
			if (request.mode === "events") return emptyEventBatch(args, "0");
			if (request.mode === "snapshot_begin") return { ...frames.begin, page_count: 129 };
			throw new Error("oversized snapshots must stop before paging");
		});
		const client = createVoiceClient(transport);
		const observed = deferred<Error>();
		const subscription = client.watch(
			() => observed.resolve(new Error("oversized snapshot was published")),
			observed.resolve,
		);
		expect(await observed.promise).toBeInstanceOf(Error);
		await subscription.close();
		expect(
			transport.calls.some((call) => {
				const request = requestFrom(call.rawRequest);
				return call.command === "voice_get_status" && request.mode === "snapshot_next";
			}),
		).toBe(false);
	});
});
