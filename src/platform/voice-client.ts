import { invoke, isTauri } from "@tauri-apps/api/core";
import {
	compareDecimalU64,
	initialVoiceReducerState,
	reduceVoiceEvent,
	sequenceRelation,
	validateReviewedTranscript,
	validateTypedText,
	type AssetCatalog,
	type AssetInstallOutcome,
	type AssetRemovalOutcome,
	type AssetStatus,
	type DecimalU64,
	type VoiceCatalogRecord,
	type VoiceEventKind,
	type VoiceEventWire,
	type VoiceInputDevice,
	type VoiceLanguage,
	type VoiceLanguageMode,
	type VoiceOwner,
	type VoicePreferences,
	type VoiceReducerState,
	type VoiceSnapshot,
} from "../domain/voice";
import {
	assembleVoiceSnapshot,
	assertRequestEnvelope,
	parseAssetCatalog,
	parseAssetInstallOutcome,
	parseAssetRemovalOutcome,
	parseVoiceCommandResult,
	parseVoiceEventBatch,
	type VoiceCommandResult,
	VoiceProtocolError,
} from "./voice-protocol";

export type VoiceCommandName =
	| "voice_get_status"
	| "voice_start"
	| "voice_finish"
	| "voice_submit_text"
	| "voice_confirm_transcript"
	| "voice_cancel"
	| "voice_clear_session"
	| "voice_select_preferences"
	| "voice_ack_intent"
	| "voice_assets_get_status"
	| "voice_assets_request_install"
	| "voice_assets_disable"
	| "voice_assets_delete"
	| "voice_assets_rollback"
	| "voice_test_output";

export interface VoiceNativeTransport {
	isNative(): boolean;
	invoke(command: VoiceCommandName, rawRequest: Uint8Array): Promise<unknown>;
}

export type VoiceCancelReason =
	| "user"
	| "surface_closed"
	| "window_hidden"
	| "window_unfocused"
	| "module_disabled"
	| "shutdown"
	| "timeout"
	| "provider_failure";

export interface VoiceSessionUpdate {
	type: "snapshot" | "events";
	snapshot: VoiceSnapshot;
	state: VoiceReducerState;
	events: VoiceEventKind[];
}

export interface VoiceSubscription {
	cancelAndWait(owner: VoiceOwner, reason: VoiceCancelReason): Promise<void>;
	close(): Promise<void>;
}

export class VoiceClientError extends Error {
	constructor(readonly kind: "unsupported" | "protocol" | "native" | "cleanup") {
		super("Voice operation could not be completed");
		this.name = "VoiceClientError";
	}
}

const tauriTransport: VoiceNativeTransport = {
	isNative: () => isTauri(),
	invoke: (command, rawRequest) => invoke<unknown>(command, rawRequest),
};

const MIN_POLL_INTERVAL_MS = 50;
const CLEANUP_DEADLINE_MS = 3_000;
const MAX_BUFFERED_EVENTS = 64;
const MAX_PUBLIC_REQUEST_BYTES = 65_536;
const requestEncoder = new TextEncoder();

function nativeError(): VoiceClientError {
	return new VoiceClientError("native");
}

function protocolError(): VoiceClientError {
	return new VoiceClientError("protocol");
}

function timerDelay(milliseconds: number, signal: AbortSignal): Promise<void> {
	if (signal.aborted) return Promise.resolve();
	let resolve!: () => void;
	const promise = new Promise<void>((resolvePromise) => {
		resolve = resolvePromise;
	});
	const timer = setTimeout(done, milliseconds);
	function done() {
		clearTimeout(timer);
		signal.removeEventListener("abort", done);
		resolve();
	}
	signal.addEventListener("abort", done, { once: true });
	return promise;
}

function eventOwnerIsActive(state: VoiceReducerState | null, owner: VoiceOwner): boolean {
	const activeOwner = state?.snapshot.owner;
	return activeOwner !== null && activeOwner !== undefined &&
		activeOwner.session_id === owner.session_id &&
		activeOwner.turn_id === owner.turn_id &&
		activeOwner.generation === owner.generation;
}

function cleanupComplete(state: VoiceReducerState | null): boolean {
	if (!state) return false;
	const { metadata } = state.snapshot;
	return !metadata.cancel_available &&
		metadata.phase !== "cancelling" &&
		["idle", "unavailable", "setup_required", "error"].includes(metadata.phase) &&
		metadata.cleanup.verified;
}

function isPromise(value: Promise<void> | null): value is Promise<void> {
	return value !== null;
}
// Completed turns enter native history snapshots rather than live event deltas.
function isTerminalOwnerRelease(event: VoiceEventWire): boolean {
	return event.owner === null &&
		event.event.type === "state_changed" &&
		event.event.phase === "idle";
}

class VoiceWatchSession implements VoiceSubscription {
	private readonly abortController = new AbortController();
	private readonly buffered: VoiceEventWire[] = [];
	private readonly cleanupWaiters = new Set<() => void>();
	private state: VoiceReducerState | null = null;
	private cursor: DecimalU64 = "0";
	private pollTask: Promise<void> | null = null;
	private snapshotTask: Promise<void> | null = null;
	private closeTask: Promise<void> | null = null;
	private pendingSnapshot = false;
	private stopped = false;
	private failure: VoiceClientError | null = null;

	constructor(
		private readonly invokeStatus: (request: Record<string, unknown>) => Promise<unknown>,
		private readonly onUpdate: (update: VoiceSessionUpdate) => void,
		private readonly onError: (error: VoiceClientError) => void,
		private readonly invokeCancel: (owner: VoiceOwner, reason: VoiceCancelReason) => Promise<VoiceCommandResult>,
	) {}

	start(): void {
		this.pollTask = this.poll();
		this.requestSnapshot();
	}

	async cancelAndWait(owner: VoiceOwner, reason: VoiceCancelReason): Promise<void> {
		if (!eventOwnerIsActive(this.state, owner)) {
			if (cleanupComplete(this.state)) return;
			throw new VoiceClientError("cleanup");
		}
		const result = await this.invokeCancel(owner, reason);
		if (!result.accepted && !cleanupComplete(this.state)) throw new VoiceClientError("cleanup");
		if (cleanupComplete(this.state)) return;
		await this.awaitCleanup();
	}

	close(): Promise<void> {
		if (this.closeTask) return this.closeTask;
		this.stopped = true;
		this.abortController.abort();
		this.notifyCleanupWaiters();
		this.closeTask = Promise.allSettled([this.pollTask, this.snapshotTask].filter(isPromise))
			.then(() => undefined);
		return this.closeTask;
	}

	private async poll(): Promise<void> {
		while (!this.stopped) {
			const requestedCursor = this.cursor;
			try {
				const raw = await this.invokeStatus({ mode: "events", after_event_sequence: requestedCursor });
				if (this.stopped) return;
				const batch = parseVoiceEventBatch(raw, requestedCursor);
				if (batch.resync_required) {
					this.cursor = batch.latest_event_sequence;
					this.buffered.length = 0;
					this.state = null;
					this.requestSnapshot();
				} else if (batch.events.length > 0) {
					this.consume(batch.events);
					const last = batch.events[batch.events.length - 1];
					if (last && compareDecimalU64(last.event_sequence, this.cursor) > 0)
						this.cursor = last.event_sequence;
				}
				if (batch.events.length === 16 && compareDecimalU64(batch.latest_event_sequence, this.cursor) > 0)
					continue;
				await timerDelay(MIN_POLL_INTERVAL_MS, this.abortController.signal);
			} catch (error) {
				if (this.stopped) return;
				this.fail(error instanceof VoiceProtocolError ? protocolError() : nativeError());
				return;
			}
		}
	}

	private consume(events: VoiceEventWire[]): void {
		if (!this.state) {
			this.bufferEvents(events);
			return;
		}
		let next = this.state;
		const semanticEvents: VoiceEventKind[] = [];
		let terminalOwnerReleased = false;
		for (const event of events) {
			const reduction = reduceVoiceEvent(next, event);
			if (reduction.outcome === "resync") {
				this.state = null;
				this.bufferEvents(events);
				this.requestSnapshot();
				return;
			}
			next = reduction.state;
			if (reduction.outcome === "applied") {
				semanticEvents.push(event.event);
				terminalOwnerReleased ||= isTerminalOwnerRelease(event);
			}
		}
		this.state = next;
		this.notifyCleanupWaiters();
		if (semanticEvents.length > 0)
			this.publish({ type: "events", snapshot: next.snapshot, state: next, events: semanticEvents });
		if (terminalOwnerReleased) this.requestSnapshot();
	}

	private bufferEvents(events: VoiceEventWire[]): void {
		if (this.buffered.length + events.length > MAX_BUFFERED_EVENTS) {
			this.buffered.length = 0;
			this.state = null;
			this.pendingSnapshot = true;
			this.requestSnapshot();
			return;
		}
		this.buffered.push(...events);
	}

	private requestSnapshot(): void {
		this.pendingSnapshot = true;
		if (this.snapshotTask || this.stopped) return;
		this.snapshotTask = this.resynchronize()
			.catch((error: unknown) => {
				if (!this.stopped)
					this.fail(error instanceof VoiceProtocolError ? protocolError() : nativeError());
			})
			.finally(() => {
				this.snapshotTask = null;
				if (this.pendingSnapshot && !this.stopped) this.requestSnapshot();
			});
	}

	private async resynchronize(): Promise<void> {
		while (this.pendingSnapshot && !this.stopped) {
			this.pendingSnapshot = false;
			const begin = await this.invokeStatus({ mode: "snapshot_begin" });
			const snapshot = await assembleVoiceSnapshot(begin, (resyncId, cursor) =>
				this.invokeStatus({ mode: "snapshot_next", resync_id: resyncId, cursor }),
			);
			if (this.stopped) return;
			let next = initialVoiceReducerState(snapshot);
			const buffered = this.buffered.splice(0);
			const replayed: VoiceEventKind[] = [];
			let terminalOwnerReleased = false;
			let needsAnotherSnapshot = false;
			for (const event of buffered) {
				if (compareDecimalU64(event.event_sequence, snapshot.event_sequence) <= 0) continue;
				const relation = sequenceRelation(next.event_sequence, event.event_sequence);
				if (relation === "duplicate" || relation === "stale") continue;
				if (relation === "gap") {
					needsAnotherSnapshot = true;
					break;
				}
				const reduction = reduceVoiceEvent(next, event);
				if (reduction.outcome === "resync") {
					needsAnotherSnapshot = true;
					break;
				}
				next = reduction.state;
				if (reduction.outcome === "applied") {
					replayed.push(event.event);
					terminalOwnerReleased ||= isTerminalOwnerRelease(event);
				}
			}
			if (needsAnotherSnapshot || compareDecimalU64(this.cursor, next.event_sequence) > 0) {
				this.buffered.unshift(...buffered);
				this.pendingSnapshot = true;
				continue;
			}
			this.state = next;
			if (compareDecimalU64(this.cursor, next.event_sequence) < 0)
				this.cursor = next.event_sequence;
			this.notifyCleanupWaiters();
			this.publish({ type: "snapshot", snapshot: next.snapshot, state: next, events: replayed });
			if (terminalOwnerReleased) this.pendingSnapshot = true;
		}
	}

	private publish(update: VoiceSessionUpdate): void {
		if (this.stopped) return;
		try {
			this.onUpdate(update);
		} catch {
			this.fail(nativeError());
		}
	}

	private fail(error: VoiceClientError): void {
		if (this.failure || this.stopped) return;
		this.failure = error;
		this.stopped = true;
		this.abortController.abort();
		this.onError(error);
		this.notifyCleanupWaiters();
	}

	private awaitCleanup(): Promise<void> {
		if (cleanupComplete(this.state)) return Promise.resolve();
		if (this.failure) return Promise.reject(new VoiceClientError("cleanup"));
		let resolve!: () => void;
		let reject!: (reason?: unknown) => void;
		const promise = new Promise<void>((resolvePromise, rejectPromise) => {
			resolve = resolvePromise;
			reject = rejectPromise;
		});
		let settled = false;
		const finish = () => {
			if (settled) return;
			settled = true;
			clearTimeout(timer);
			this.cleanupWaiters.delete(check);
			if (cleanupComplete(this.state)) resolve();
			else reject(new VoiceClientError("cleanup"));
		};
		const check = () => {
			if (cleanupComplete(this.state) || this.failure || this.stopped) finish();
		};
		const timer = setTimeout(finish, CLEANUP_DEADLINE_MS);
		this.cleanupWaiters.add(check);
		return promise;
	}

	private notifyCleanupWaiters(): void {
		for (const waiter of this.cleanupWaiters) waiter();
	}
}

export interface VoiceClient {
	isSupported(): boolean;
	start(languageMode: VoiceLanguageMode, inputDevice: VoiceInputDevice | null): Promise<VoiceCommandResult>;
	finish(owner: VoiceOwner): Promise<VoiceCommandResult>;
	submitText(text: string, languageMode: VoiceLanguageMode): Promise<VoiceCommandResult>;
	confirmTranscript(owner: VoiceOwner, text: string, language: VoiceLanguage): Promise<VoiceCommandResult>;
	cancel(owner: VoiceOwner, reason: VoiceCancelReason): Promise<VoiceCommandResult>;
	clearSession(): Promise<VoiceCommandResult>;
	selectPreferences(preferences: VoicePreferences): Promise<VoiceCommandResult>;
	acknowledgeIntent(owner: VoiceOwner, intentId: DecimalU64, accepted: boolean): Promise<VoiceCommandResult>;
	assetsGetStatus(): Promise<AssetCatalog>;
	requestAssetInstall(asset: AssetStatus): Promise<AssetInstallOutcome>;
	disableAsset(asset: AssetStatus): Promise<AssetCatalog>;
	deleteAsset(asset: AssetStatus): Promise<AssetRemovalOutcome>;
	rollbackAsset(asset: AssetStatus): Promise<AssetCatalog>;
	testOutput(voice: VoiceCatalogRecord): Promise<VoiceCommandResult>;
	watch(onUpdate: (update: VoiceSessionUpdate) => void, onError: (error: VoiceClientError) => void): VoiceSubscription;
}

export function createVoiceClient(transport: VoiceNativeTransport = tauriTransport): VoiceClient {
	function supported(): boolean {
		try {
			return transport.isNative();
		} catch {
			return false;
		}
	}

	async function invokeNative(command: VoiceCommandName, fields: Record<string, unknown>): Promise<unknown> {
		if (!supported()) throw new VoiceClientError("unsupported");
		const envelope = { request: { protocol_version: 1, ...fields } };
		assertRequestEnvelope(envelope);
		const serialized = JSON.stringify(envelope);
		if (typeof serialized !== "string") throw protocolError();
		const rawRequest = requestEncoder.encode(serialized);
		if (rawRequest.byteLength > MAX_PUBLIC_REQUEST_BYTES) throw protocolError();
		try {
			return await transport.invoke(command, rawRequest);
		} catch {
			throw nativeError();
		}
	}

	async function result(command: VoiceCommandName, fields: Record<string, unknown>): Promise<VoiceCommandResult> {
		try {
			return parseVoiceCommandResult(await invokeNative(command, fields));
		} catch (error) {
			if (error instanceof VoiceClientError) throw error;
			throw protocolError();
		}
	}

	async function requestAssetAction(
		command: "voice_assets_disable" | "voice_assets_delete",
		asset: AssetStatus,
	): Promise<unknown> {
		if (!asset.asset_id) throw protocolError();
		return invokeNative(command, { asset_id: asset.asset_id });
	}

	const client: VoiceClient = {
		isSupported: supported,
		start: (languageMode, inputDevice) => result("voice_start", {
			language_mode: languageMode,
			input_device_handle: inputDevice?.handle ?? null,
		}),
		finish: (owner) => result("voice_finish", { owner }),
		submitText: (text, languageMode) => {
			if (!validateTypedText(text).ok) return Promise.reject(new VoiceClientError("protocol"));
			return result("voice_submit_text", { text, language_mode: languageMode });
		},
		confirmTranscript: (owner, text, language) => {
			if (!validateReviewedTranscript(text).ok) return Promise.reject(new VoiceClientError("protocol"));
			return result("voice_confirm_transcript", { owner, text, language });
		},
		cancel: (owner, reason) => result("voice_cancel", { owner, reason }),
		clearSession: () => result("voice_clear_session", {}),
		selectPreferences: (preferences) => result("voice_select_preferences", { ...preferences }),
		acknowledgeIntent: (owner, intentId, accepted) =>
			result("voice_ack_intent", { owner, intent_id: intentId, accepted }),
		assetsGetStatus: async () => {
			try {
				return parseAssetCatalog(await invokeNative("voice_assets_get_status", {}));
			} catch (error) {
				if (error instanceof VoiceClientError) throw error;
				throw protocolError();
			}
		},
		requestAssetInstall: async (asset) => {
			if (asset.status !== "awaiting_approval") throw new VoiceClientError("unsupported");
			try {
				return parseAssetInstallOutcome(await invokeNative("voice_assets_request_install", {
					asset_id: asset.asset_id,
				}));
			} catch (error) {
				if (error instanceof VoiceClientError) throw error;
				throw protocolError();
			}
		},
		disableAsset: async (asset) => {
			try {
				return parseAssetCatalog(await requestAssetAction("voice_assets_disable", asset));
			} catch (error) {
				if (error instanceof VoiceClientError) throw error;
				throw protocolError();
			}
		},
		deleteAsset: async (asset) => {
			try {
				return parseAssetRemovalOutcome(await requestAssetAction("voice_assets_delete", asset));
			} catch (error) {
				if (error instanceof VoiceClientError) throw error;
				throw protocolError();
			}
		},
		rollbackAsset: async (asset) => {
			if (!asset.asset_id || !asset.asset_disclosure.version) throw protocolError();
			try {
				return parseAssetCatalog(await invokeNative("voice_assets_rollback", {
					asset_id: asset.asset_id,
					version_id: asset.asset_disclosure.version,
				}));
			} catch (error) {
				if (error instanceof VoiceClientError) throw error;
				throw protocolError();
			}
		},
		testOutput: (voice) => result("voice_test_output", {
			language: voice.language,
			voice_id: voice.id,
		}),
		watch: (onUpdate, onError) => {
			if (!supported()) {
				const error = new VoiceClientError("unsupported");
				let closed = false;
				queueMicrotask(() => {
					if (!closed) onError(error);
				});
				return {
					close: async () => { closed = true; },
					cancelAndWait: async () => { throw error; },
				};
			}
			const session = new VoiceWatchSession(
				(request) => invokeNative("voice_get_status", request),
				onUpdate,
				onError,
				(owner, reason) => result("voice_cancel", { owner, reason }),
			);
			session.start();
			return session;
		},
	};
	return client;
}

export const voiceClient = createVoiceClient();
