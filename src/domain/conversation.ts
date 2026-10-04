export type ConversationAction = "diagnostic" | "focus";

export interface ConversationRequest {
	text: string;
}

export interface ConversationReply {
	text: string;
	action?: ConversationAction;
}

export interface ConversationProviderMetadata {
	id: string;
	name: string;
	mode: "scripted" | "local-model" | "remote-model";
	locality: "local" | "remote";
}

export interface ConversationProvider {
	metadata: ConversationProviderMetadata;
	respond(
		request: ConversationRequest,
		options: { signal: AbortSignal },
	): Promise<ConversationReply>;
}

export class ConversationRuntime {
	readonly provider: ConversationProvider;

	constructor(provider: ConversationProvider) {
		this.provider = provider;
	}

	async respond(
		draft: string,
		signal: AbortSignal,
	): Promise<ConversationReply | null> {
		const text = draft.trim().slice(0, 500);
		if (!text) return null;
		return this.provider.respond({ text }, { signal });
	}
}

function abortError(): DOMException {
	return new DOMException("The conversation request was cancelled", "AbortError");
}

function wait(delayMs: number, signal: AbortSignal): Promise<void> {
	if (signal.aborted) return Promise.reject(abortError());
	return new Promise((resolve, reject) => {
		const timer = setTimeout(() => {
			signal.removeEventListener("abort", onAbort);
			resolve();
		}, delayMs);
		function onAbort() {
			clearTimeout(timer);
			reject(abortError());
		}
		signal.addEventListener("abort", onAbort, { once: true });
	});
}

function scriptedReply(text: string): ConversationReply {
	if (/\b(scan|diagnostic|diagnostics)\b/i.test(text)) {
		return {
			text: "Starting the diagnostic simulation. Four illustrative modules, one safe sequence. This does not inspect your hardware or contact any services.",
			action: "diagnostic",
		};
	}
	if (/\bfocus\b/i.test(text)) {
		return {
			text: "Returning to the compact core. Your tasks and diagnostics remain available in Workspace.",
			action: "focus",
		};
	}
	if (/\b(status|briefing|hello|hi)\b/i.test(text)) {
		return {
			text: "The local interface is online. Your saved settings, and checklist are real. Diagnostics and this reply are scripted. No AI provider is connected.",
		};
	}
	return {
		text: "I’m in interface preview mode, not connected to an AI model. Try “status” for a scripted briefing, “run a diagnostic” for a safe simulation, or “focus” to simplify the workspace. Your messages stay in this session.",
	};
}

export function createScriptedConversationProvider({
	delayMs = 850,
}: {
	delayMs?: number;
} = {}): ConversationProvider {
	return {
		metadata: {
			id: "core.scripted-preview",
			name: "Scripted preview",
			mode: "scripted",
			locality: "local",
		},
		async respond(request, { signal }) {
			await wait(delayMs, signal);
			return scriptedReply(request.text);
		},
	};
}
