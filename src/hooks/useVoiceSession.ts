import { useCallback, useEffect, useRef, useState } from "react";
import { compareDecimalU64, isDecimalU64 } from "../domain/voice";
import type {
	AssetCatalog,
	AssetInstallOutcome,
	AssetRemovalOutcome,
	AssetStatus,
	DecimalU64,
	VoiceCatalogRecord,
	VoiceInputDevice,
	VoiceLanguage,
	VoiceLanguageMode,
	VoiceOwner,
	VoicePreferences,
	VoiceReducerState,
} from "../domain/voice";
import {
	voiceClient,
	type VoiceCancelReason,
	type VoiceClient,
	type VoiceClientError,
	type VoiceSubscription,
} from "../platform/voice-client";
import type { VoiceCommandResult } from "../platform/voice-protocol";

function isVoiceOwner(value: unknown): value is VoiceOwner {
	if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
	if (!("session_id" in value) || !("turn_id" in value) || !("generation" in value)) return false;
	return typeof value.session_id === "string" && value.session_id.length > 0 &&
		typeof value.turn_id === "string" && value.turn_id.length > 0 &&
		isDecimalU64(value.generation);
}
type OperationError = VoiceClientError["kind"] | null;
type SessionStatus = "idle" | "unsupported" | "connecting" | "ready" | "failed";

function sessionCleanupComplete(state: VoiceReducerState | null): boolean {
	if (!state) return false;
	const { metadata } = state.snapshot;
	return !metadata.cancel_available &&
		metadata.phase !== "cancelling" &&
		["idle", "unavailable", "setup_required", "error"].includes(metadata.phase) &&
		metadata.cleanup.verified;
}


function stateOwns(state: VoiceReducerState | null, owner: VoiceOwner): boolean {
	const current = state?.snapshot.owner;
	return current !== null && current !== undefined &&
		current.session_id === owner.session_id &&
		current.turn_id === owner.turn_id &&
		current.generation === owner.generation;
}

export interface VoiceSessionController {
	supported: boolean;
	status: SessionStatus;
	state: VoiceReducerState | null;
	busy: boolean;
	error: OperationError;
	start(languageMode: VoiceLanguageMode, inputDevice: VoiceInputDevice | null): Promise<VoiceCommandResult | null>;
	finish(): Promise<VoiceCommandResult | null>;
	submitText(text: string, languageMode: VoiceLanguageMode): Promise<VoiceCommandResult | null>;
	confirmTranscript(text: string, language: VoiceLanguage): Promise<VoiceCommandResult | null>;
	cancelTurn(reason?: VoiceCancelReason): Promise<boolean>;
	clearSession(): Promise<VoiceCommandResult | null>;
	selectPreferences(preferences: VoicePreferences): Promise<VoiceCommandResult | null>;
	acknowledgeIntent(owner: VoiceOwner, intentId: DecimalU64, accepted: boolean): Promise<VoiceCommandResult | null>;
	assetsGetStatus(): Promise<AssetCatalog | null>;
	requestAssetInstall(asset: AssetStatus): Promise<AssetInstallOutcome | null>;
	disableAsset(asset: AssetStatus): Promise<AssetCatalog | null>;
	deleteAsset(asset: AssetStatus): Promise<AssetRemovalOutcome | null>;
	rollbackAsset(asset: AssetStatus): Promise<AssetCatalog | null>;
	testOutput(voice: VoiceCatalogRecord): Promise<VoiceCommandResult | null>;
	closeSurface(reason?: VoiceCancelReason): Promise<boolean>;
}

export function useVoiceSession(
	active: boolean,
	onSurfaceClosed: () => void,
	client: VoiceClient = voiceClient,
): VoiceSessionController {
	const [state, setState] = useState<VoiceReducerState | null>(null);
	const [status, setStatus] = useState<SessionStatus>("idle");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<OperationError>(null);
	const stateRef = useRef<VoiceReducerState | null>(null);
	const subscriptionRef = useRef<VoiceSubscription | null>(null);
	const commandRef = useRef<Promise<unknown> | null>(null);
	const commandOwnerRef = useRef<VoiceOwner | null>(null);
	const closingRef = useRef(false);
	const closePromiseRef = useRef<Promise<boolean> | null>(null);
	const busyRef = useRef(false);
	const stateWaiters = useRef(new Set<() => void>());
	const onSurfaceClosedRef = useRef(onSurfaceClosed);
	const closeSurfaceRef = useRef<(reason: VoiceCancelReason) => Promise<boolean>>(() => Promise.resolve(false));

	useEffect(() => {
		onSurfaceClosedRef.current = onSurfaceClosed;
	}, [onSurfaceClosed]);

	const notifyStateWaiters = useCallback(() => {
		for (const notify of stateWaiters.current) notify();
	}, []);

	const waitForState = useCallback((predicate: () => boolean): Promise<void> => {
		if (predicate()) return Promise.resolve();
		let resolve!: () => void;
		let reject!: (reason?: unknown) => void;
		const promise = new Promise<void>((resolvePromise, rejectPromise) => {
			resolve = resolvePromise;
			reject = rejectPromise;
		});
		let settled = false;
		const check = () => {
			if (!predicate() || settled) return;
			settled = true;
			clearTimeout(timer);
			stateWaiters.current.delete(check);
			resolve();
		};
		const timer = setTimeout(() => {
			if (settled) return;
			settled = true;
			stateWaiters.current.delete(check);
			reject(new Error("Voice state did not reach the required native owner."));
		}, 3_000);
		stateWaiters.current.add(check);
		return promise;
	}, []);

	const reportError = useCallback((next: OperationError) => {
		setError(next);
		if (next === "unsupported") setStatus("unsupported");
		else if (next !== null) setStatus("failed");
	}, []);

	const runCommand = useCallback(async <T,>(action: () => Promise<T>): Promise<T | null> => {
		if (busyRef.current || closingRef.current) return null;
		busyRef.current = true;
		setBusy(true);
		setError(null);
		const command = action();
		commandRef.current = command;
		try {
			const result = await command;
			if (result && typeof result === "object" && "accepted" in result && result.accepted === false)
				reportError("native");
			if (result && typeof result === "object" && "transfer" in result) {
				const transfer = result.transfer;
				if (transfer && typeof transfer === "object" && "owner" in transfer && isVoiceOwner(transfer.owner))
					commandOwnerRef.current = transfer.owner;
			}
			return result;
		} catch (caught) {
			const kind = caught instanceof Error && "kind" in caught &&
				(caught.kind === "unsupported" || caught.kind === "protocol" || caught.kind === "native" || caught.kind === "cleanup")
				? caught.kind
				: "native";
			reportError(kind);
			return null;
		} finally {
			commandRef.current = null;
			busyRef.current = closingRef.current;
			setBusy(closingRef.current);
		}
	}, [reportError]);

	const cancelTurn = useCallback(async (reason: VoiceCancelReason = "user"): Promise<boolean> => {
		const subscription = subscriptionRef.current;
		const owner = stateRef.current?.snapshot.owner ?? commandOwnerRef.current;
		if (!subscription || !owner || closingRef.current) return false;
		busyRef.current = true;
		setBusy(true);
		setError(null);
		try {
			await subscription.cancelAndWait(owner, reason);
			commandOwnerRef.current = null;
			return true;
		} catch {
			reportError("cleanup");
			return false;
		} finally {
			busyRef.current = closingRef.current;
			setBusy(closingRef.current);
		}
	}, [reportError]);

	const closeSurface = useCallback((reason: VoiceCancelReason = "surface_closed"): Promise<boolean> => {
		if (closePromiseRef.current) return closePromiseRef.current;
		closingRef.current = true;
		busyRef.current = true;
		setBusy(true);
		const closing = (async () => {
			try {
				const pendingCommand = commandRef.current;
				if (pendingCommand) await pendingCommand;
				const subscription = subscriptionRef.current;
				const current = stateRef.current;
				const owner = current?.snapshot.owner ?? commandOwnerRef.current;
				if (subscription && owner) {
					let latest = current;
					if (!stateOwns(current, owner)) {
						const baselineSequence = current?.event_sequence ?? "0";
						await waitForState(() => {
							const observed = stateRef.current;
							return stateOwns(observed, owner) ||
								(observed !== null &&
									compareDecimalU64(observed.event_sequence, baselineSequence) > 0 &&
									sessionCleanupComplete(observed));
						});
						latest = stateRef.current;
					}
					if (stateOwns(latest, owner) && !sessionCleanupComplete(latest))
						await subscription.cancelAndWait(owner, reason);
					else if (!sessionCleanupComplete(latest))
						throw new Error("Native cleanup was not confirmed.");
				}
				await subscription?.close();
				if (subscriptionRef.current === subscription) subscriptionRef.current = null;
				commandOwnerRef.current = null;
				stateRef.current = null;
				setState(null);
				setStatus("idle");
				setError(null);
				return true;
			} catch {
				reportError("cleanup");
				return false;
			} finally {
				busyRef.current = false;
				setBusy(false);
				closingRef.current = false;
				closePromiseRef.current = null;
			}
		})();
		closePromiseRef.current = closing;
		return closing;
	}, [reportError, waitForState]);

	useEffect(() => {
		closeSurfaceRef.current = closeSurface;
	}, [closeSurface]);

	useEffect(() => {
		if (!active) {
			setStatus("idle");
			return;
		}
		if (!client.isSupported()) {
			setStatus("unsupported");
			return;
		}
		setStatus("connecting");
		setError(null);
		let disposed = false;
		const subscription = client.watch((update) => {
			if (disposed) return;
			stateRef.current = update.state;
			setState(update.state);
			setStatus("ready");
			if (update.state.snapshot.owner) commandOwnerRef.current = update.state.snapshot.owner;
			else if (sessionCleanupComplete(update.state)) commandOwnerRef.current = null;
			notifyStateWaiters();
		}, (caught) => {
			if (disposed) return;
			reportError(caught.kind);
		});
		subscriptionRef.current = subscription;
		const onPageHide = () => {
			void closeSurfaceRef.current("window_hidden").then((closed) => {
				if (closed) onSurfaceClosedRef.current();
			});
		};
		const onVisibility = () => {
			if (document.visibilityState === "hidden") onPageHide();
		};
		window.addEventListener("pagehide", onPageHide);
		document.addEventListener("visibilitychange", onVisibility);
		return () => {
			disposed = true;
			window.removeEventListener("pagehide", onPageHide);
			document.removeEventListener("visibilitychange", onVisibility);
			void closeSurfaceRef.current("surface_closed");
		};
	}, [active, client, notifyStateWaiters, reportError]);

	const start = useCallback((languageMode: VoiceLanguageMode, inputDevice: VoiceInputDevice | null) =>
		runCommand(() => client.start(languageMode, inputDevice)), [client, runCommand]);
	const finish = useCallback(() => {
		const owner = stateRef.current?.snapshot.owner ?? commandOwnerRef.current;
		return owner ? runCommand(() => client.finish(owner)) : Promise.resolve(null);
	}, [client, runCommand]);
	const submitText = useCallback((text: string, languageMode: VoiceLanguageMode) =>
		runCommand(() => client.submitText(text, languageMode)), [client, runCommand]);
	const confirmTranscript = useCallback((text: string, language: VoiceLanguage) => {
		const owner = stateRef.current?.snapshot.owner ?? commandOwnerRef.current;
		return owner ? runCommand(() => client.confirmTranscript(owner, text, language)) : Promise.resolve(null);
	}, [client, runCommand]);
	const clearSession = useCallback(() => runCommand(() => client.clearSession()), [client, runCommand]);
	const selectPreferences = useCallback((preferences: VoicePreferences) =>
		runCommand(() => client.selectPreferences(preferences)), [client, runCommand]);
	const acknowledgeIntent = useCallback((owner: VoiceOwner, intentId: DecimalU64, accepted: boolean) =>
		runCommand(() => client.acknowledgeIntent(owner, intentId, accepted)), [client, runCommand]);
	const assetsGetStatus = useCallback(async () => {
		try {
			return await client.assetsGetStatus();
		} catch (caught) {
			const kind = caught instanceof Error && "kind" in caught &&
				(caught.kind === "unsupported" || caught.kind === "protocol" || caught.kind === "native" || caught.kind === "cleanup")
				? caught.kind
				: "native";
			reportError(kind);
			return null;
		}
	}, [client, reportError]);
	const requestAssetInstall = useCallback((asset: AssetStatus) =>
		runCommand(() => client.requestAssetInstall(asset)), [client, runCommand]);
	const disableAsset = useCallback((asset: AssetStatus) =>
		runCommand(() => client.disableAsset(asset)), [client, runCommand]);
	const deleteAsset = useCallback((asset: AssetStatus) =>
		runCommand(() => client.deleteAsset(asset)), [client, runCommand]);
	const rollbackAsset = useCallback((asset: AssetStatus) =>
		runCommand(() => client.rollbackAsset(asset)), [client, runCommand]);
	const testOutput = useCallback((voice: VoiceCatalogRecord) =>
		runCommand(() => client.testOutput(voice)), [client, runCommand]);

	return {
		supported: client.isSupported(),
		status,
		state,
		busy,
		error,
		start,
		finish,
		submitText,
		confirmTranscript,
		cancelTurn,
		clearSession,
		selectPreferences,
		acknowledgeIntent,
		assetsGetStatus,
		requestAssetInstall,
		disableAsset,
		deleteAsset,
		rollbackAsset,
		testOutput,
		closeSurface,
	};
}
