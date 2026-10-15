import type { Page } from "@playwright/test";
declare global {
	interface Window {
		isTauri: boolean;
		__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__: {
			calls: Array<{ command: string; request: Record<string, unknown> }>;
			releaseSnapshot: () => void;
		};
	}
}

/**
 * Explicit browser-only native protocol driver. This fixture exercises the renderer
 * contract; it is not a provider, a capability substitute, or evidence of native readiness.
 */
export async function installNativeProtocolTestDriver(
	page: Page,
	options: { holdSnapshot?: boolean } = {},
) {
	await page.addInitScript((holdSnapshot: boolean) => {
		let releaseSnapshot: (() => void) | null = null;
		const snapshotGate = holdSnapshot
			? new Promise<void>((resolve) => { releaseSnapshot = resolve; })
			: Promise.resolve();
		const calls: Array<{ command: string; request: Record<string, unknown> }> = [];
		const eventLog: Array<Record<string, unknown>> = [];
		const history: Array<Record<string, unknown>> = [];
		const transcript = "A local test transcript.";
		const answer = "A local test response.";
		const owner = {
			session_id: "protocol-test-session",
			turn_id: "protocol-test-turn",
			generation: "18446744073709551615",
		};
		let sequence = 0n;
		let revision = 0n;
		let activeOwner: typeof owner | null = null;
		let phase = "idle";
		let transcriptSequence = 0n;
		let answerSequence = 0n;
		let preferences: {
			language_mode: "auto" | "en" | "tr" | null;
			input_device_handle: string | null;
			voice_ids: { tr: string | null; en: string | null };
			spoken_reply: boolean;
			review_before_send: boolean;
		} = {
			language_mode: "auto",
			input_device_handle: null,
			voice_ids: { tr: null, en: null },
			spoken_reply: false,
			review_before_send: false,
		};
		const emptyCleanup = { stopped: true, verified: true, code: "no_resources_owned" };
		const activeCleanup = { stopped: false, verified: false, code: "no_resources_owned" };
		const doneCleanup = { stopped: true, verified: true, code: "verified" };

		function metadata() {
			return {
				session_id: "protocol-test-session",
				phase,
				preferences: {
					...preferences,
					voice_ids: { ...preferences.voice_ids },
				},
				resolved_language: null,
				readiness: {
					capture: { state: "setup_required", code: "permission_required" },
					asr: { state: "unavailable", code: "owner_trust_setup_required" },
					router: { state: "setup_required", code: "runtime_build_required" },
					llm: { state: "unavailable", code: "owner_trust_setup_required" },
					speech: { state: "unavailable", code: "provider_unavailable" },
					routine_tasks: { completed: "0", without_llm: "0" },
					product_ready: false,
				},
				voice_catalog: { revision: "0" },
				cancel_available: activeOwner !== null,
				cleanup: activeOwner === null ? (phase === "idle" ? emptyCleanup : doneCleanup) : activeCleanup,
				route_kind: phase === "thinking" || phase === "speaking" ? "local_model" : null,
				capture_elapsed_ms: phase === "listening" ? 800 : null,
				output_progress: { queued_segments: 0, completed_segments: 0, active: false },
				next_transcript_sequence: transcriptSequence.toString(),
				next_answer_sequence: answerSequence.toString(),
				notices: [],
				error: null,
			};
		}

		function nextEvent(event: Record<string, unknown>, eventOwner = activeOwner) {
			sequence += 1n;
			revision += 1n;
			eventLog.push({
				protocol_version: 1,
				event_sequence: sequence.toString(),
				revision: revision.toString(),
				owner: eventOwner,
				event,
			});
		}

		function stateEvent(nextPhase: string, nextOwner = activeOwner, canCancel = nextOwner !== null) {
			phase = nextPhase;
			activeOwner = nextOwner;
			nextEvent({
				type: "state_changed",
				phase: nextPhase,
				route_kind: ["thinking", "speaking"].includes(nextPhase) ? "local_model" : null,
				cancel_available: canCancel,
				cleanup: nextOwner === null ? doneCleanup : activeCleanup,
			}, nextOwner);
		}

		function commandResult(accepted: boolean, transferOwner = activeOwner) {
			return {
				accepted,
				code: accepted ? "accepted" : "no_change",
				transfer: accepted ? {
					resync_id: "0",
					event_sequence: sequence.toString(),
					revision: revision.toString(),
					owner: transferOwner,
				} : null,
			};
		}

		function snapshotWithoutCursors() {
			return {
				metadata: metadata(),
				owner: activeOwner ? { ...activeOwner } : null,
				history: history.map((message) => ({ ...message })),
				pending: [],
				input_devices: [],
				voices: [],
			};
		}

		function snapshotPages(messages: Array<Record<string, unknown>>) {
			const pages: Array<Array<Record<string, unknown>>> = [];
			let totalTextBytes = 0;
			for (let messageIndex = 0; messageIndex < messages.length; messageIndex += 1) {
				const message = messages[messageIndex];
				if (!message) continue;
				const text = String(message.text);
				const totalBytes = new TextEncoder().encode(text).byteLength;
				pages.push([{
					kind: "history_message",
					message_index: messageIndex,
					id: message.id,
					role: message.role,
					language: message.language,
					source: message.source,
					total_bytes: totalBytes,
					chunk_count: 1,
				}]);
				pages.push([{
					kind: "history_chunk",
					message_index: messageIndex,
					byte_offset: 0,
					total_bytes: totalBytes,
					chunk_index: 0,
					chunk_count: 1,
					text,
				}]);
				totalTextBytes += totalBytes;
			}
			return { pages, totalTextBytes };
		}

		let activeSnapshotTransfer: {
			header: Record<string, unknown>;
			metadata: Record<string, unknown>;
			pages: Array<Array<Record<string, unknown>>>;
		} | null = null;

		async function snapshotHeader() {
			await snapshotGate;
			const snapshot = snapshotWithoutCursors();
			const { pages, totalTextBytes } = snapshotPages(snapshot.history);
			const eventSequence = sequence.toString();
			const revisionWatermark = revision.toString();
			const digestMaterial = {
				version: "saturday.voice.snapshot.digest.v1",
				metadata: snapshot.metadata,
				owner: snapshot.owner,
				history: snapshot.history.map((message) => ({ ...message, text: "" })),
				pending: [],
				input_devices: [],
				voices: [],
			};
			const serialized = JSON.stringify(digestMaterial);
			const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(serialized)));
			const digestSha256 = Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
			return {
				header: {
					protocol_version: 1,
					event_sequence: eventSequence,
					resync_id: "0",
					revision: revisionWatermark,
					owner: snapshot.owner,
					page_count: pages.length,
					total_text_bytes: totalTextBytes,
					total_metadata_bytes: new TextEncoder().encode(serialized).byteLength,
					digest_sha256: digestSha256,
				},
				metadata: snapshot.metadata,
				pages,
			};
		}


		function decodeRequest(value: unknown) {
			let bytes: Uint8Array;
			if (value instanceof Uint8Array) bytes = value;
			else if (value instanceof ArrayBuffer) bytes = new Uint8Array(value);
			else if (Array.isArray(value) && value.every((item) => Number.isInteger(item) && item >= 0 && item <= 255))
				bytes = Uint8Array.from(value as number[]);
			else throw new Error("Protocol test driver expected raw UTF-8 request bytes");
			const parsed: unknown = JSON.parse(new TextDecoder().decode(bytes));
			if (typeof parsed !== "object" || parsed === null || !("request" in parsed))
				throw new Error("Protocol test driver received an invalid request envelope");
			const request = (parsed as { request: unknown }).request;
			if (typeof request !== "object" || request === null || Array.isArray(request))
				throw new Error("Protocol test driver received an invalid request body");
			return request as Record<string, unknown>;
		}

		async function invoke(command: string, rawRequest: unknown) {
			const request = decodeRequest(rawRequest);
			calls.push({ command, request });
			if (command === "voice_get_status") {
				if (request.mode === "snapshot_begin") {
					activeSnapshotTransfer = await snapshotHeader();
					return {
						frame: "begin",
						...activeSnapshotTransfer.header,
						metadata: activeSnapshotTransfer.metadata,
					};
				}
				if (request.mode === "snapshot_next") {
					const transfer = activeSnapshotTransfer;
					if (!transfer) throw new Error("Snapshot continuation has no baseline transfer");
					const { header, pages } = transfer;
					const cursor = Number(request.cursor);
					const pageCount = Number(header.page_count);
					if (!Number.isSafeInteger(cursor) || cursor < 0 || cursor > pageCount)
						throw new Error("Snapshot page cursor is out of range");
					if (cursor === pageCount) return { frame: "end", ...header };
					const items = pages[cursor];
					if (!items) throw new Error("Snapshot page is missing");
					return {
						frame: "page",
						protocol_version: 1,
						event_sequence: header.event_sequence,
						resync_id: header.resync_id,
						revision: header.revision,
						owner: header.owner,
						index: cursor,
						section: "history",
						items,
					};
				}
				const after = BigInt(String(request.after_event_sequence));
				const events = eventLog.filter((event) => BigInt(String(event.event_sequence)) > after);
				return {
					protocol_version: 1,
					after_event_sequence: request.after_event_sequence,
					latest_event_sequence: sequence.toString(),
					resync_required: false,
					events,
				};
			}
			if (command === "voice_select_preferences") {
				preferences = {
					language_mode:
						request.language_mode === "auto" || request.language_mode === "en" || request.language_mode === "tr"
							? request.language_mode
							: null,
					input_device_handle: typeof request.input_device_handle === "string" ? request.input_device_handle : null,
					voice_ids: request.voice_ids as typeof preferences.voice_ids,
					spoken_reply: request.spoken_reply === true,
					review_before_send: request.review_before_send === true,
				};
				return commandResult(true, null);
			}
			if (command === "voice_start") {
				activeOwner = { ...owner, turn_id: `protocol-test-turn-${sequence + 1n}` };
				stateEvent("listening", activeOwner);
				return commandResult(true, activeOwner);
			}
			if (command === "voice_finish") {
				stateEvent("transcribing");
				nextEvent({
					type: "transcript_delta",
					sequence: transcriptSequence.toString(),
					text: transcript,
					final_chunk: true,
					language: "en",
					source: "speech",
				});
				transcriptSequence += 1n;
				stateEvent("needs_review");
				return commandResult(true);
			}
			if (command === "voice_submit_text" || command === "voice_confirm_transcript") {
				if (command === "voice_submit_text") {
					history.push({
						id: `protocol-test-user-${revision + 1n}`,
						role: "user",
						language: request.language_mode === "tr" ? "tr" : "en",
						source: "typed",
						text: String(request.text),
					});
				}
				activeOwner = { ...owner, turn_id: `protocol-test-turn-${sequence + 1n}` };
				stateEvent("thinking", activeOwner);
				if (command === "voice_submit_text") {
					history.push({
						id: `protocol-test-assistant-${revision + 1n}`,
						role: "assistant",
						language: request.language_mode === "tr" ? "tr" : "en",
						source: "typed",
						text: answer,
					});
				}
				nextEvent({
					type: "answer_delta",
					sequence: answerSequence.toString(),
					text: answer,
					final_chunk: true,
					language: request.language === "tr" || request.language_mode === "tr" ? "tr" : "en",
					partial: false,
				});
				answerSequence += 1n;
				stateEvent("idle", null, false);
				return commandResult(true, null);
			}
			if (command === "voice_cancel") {
				stateEvent("idle", null, false);
				return commandResult(true, null);
			}
			if (command === "voice_clear_session") {
				history.length = 0;
				return commandResult(true, null);
			}
			return commandResult(true, null);
		}

		Object.defineProperty(window, "isTauri", { configurable: false, value: true });
		Object.defineProperty(window, "__TAURI_INTERNALS__", {
			configurable: false,
			value: { invoke },
		});
		Object.defineProperty(window, "__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__", {
			configurable: false,
			value: { calls, releaseSnapshot: () => releaseSnapshot?.() },
		});
	}, options.holdSnapshot ?? false);
}
