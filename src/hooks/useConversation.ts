import { useCallback, useEffect, useRef, useState } from "react";
import type { CoreState } from "../components/Core";
import {
	ConversationRuntime,
	createScriptedConversationProvider,
	type ConversationAction,
} from "../domain/conversation";

export type DemoAction = ConversationAction;

export interface Message {
	id: string;
	role: "user" | "assistant";
	text: string;
	sample?: boolean;
}

const defaultRuntime = new ConversationRuntime(
	createScriptedConversationProvider(),
);

export function useConversation(
	onAction: (action: DemoAction) => void,
	runtime: ConversationRuntime = defaultRuntime,
) {
	const [messages, setMessages] = useState<Message[]>([
		{
			id: "welcome",
			role: "assistant",
			text: "Good to see you, Rohtolos. The interface is yours. This is a scripted preview of the conversation experience, ready for a real intelligence layer when you are.",
		},
	]);
	const [phase, setPhase] = useState<CoreState>("standby");
	const [pending, setPending] = useState(false);
	const busy = useRef(false);
	const generation = useRef(0);
	const activeRequest = useRef<AbortController | null>(null);
	const phaseTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
	const action = useRef(onAction);

	useEffect(() => {
		action.current = onAction;
	}, [onAction]);

	const append = useCallback(
		(message: Omit<Message, "id">) =>
			setMessages((list) =>
				[...list, { ...message, id: crypto.randomUUID() }].slice(-40),
			),
		[],
	);

	const clearPhaseTimer = useCallback(() => {
		if (phaseTimer.current !== null) clearTimeout(phaseTimer.current);
		phaseTimer.current = null;
	}, []);

	const endOwnership = useCallback(() => {
		generation.current += 1;
		activeRequest.current?.abort();
		activeRequest.current = null;
		clearPhaseTimer();
	}, [clearPhaseTimer]);

	useEffect(
		() => () => {
			endOwnership();
		},
		[endOwnership],
	);

	const send = useCallback(
		(draft: string, sample = false) => {
			const text = draft.trim().slice(0, 500);
			if (!text || busy.current) return false;

			endOwnership();
			const owner = generation.current;
			const controller = new AbortController();
			activeRequest.current = controller;
			busy.current = true;
			setPending(true);
			setPhase("processing");
			append({ role: "user", text, sample });

			void runtime
				.respond(text, controller.signal)
				.then((reply) => {
					if (
						generation.current !== owner ||
						controller.signal.aborted ||
						!reply
					)
						return;
					append({ role: "assistant", text: reply.text });
					if (reply.action) action.current(reply.action);
					activeRequest.current = null;
					busy.current = false;
					setPending(false);
					setPhase("responding");
					phaseTimer.current = setTimeout(() => {
						if (generation.current === owner) setPhase("standby");
					}, 1400);
				})
				.catch((error: unknown) => {
					if (
						controller.signal.aborted ||
						generation.current !== owner ||
						(error instanceof DOMException && error.name === "AbortError")
					)
						return;
					activeRequest.current = null;
					busy.current = false;
					setPending(false);
					setPhase("standby");
					append({
						role: "assistant",
						text: "The current conversation provider could not respond. No action was taken.",
					});
				});

			return true;
		},
		[append, endOwnership, runtime],
	);

	const cancel = useCallback(() => {
		const wasBusy = busy.current;
		endOwnership();
		busy.current = false;
		setPending(false);
		setPhase("standby");
		if (wasBusy) {
			append({
				role: "assistant",
				text: "The pending preview was cancelled when you closed the conversation.",
			});
		}
	}, [append, endOwnership]);

	return {
		messages,
		phase,
		pending,
		send,
		cancel,
		provider: runtime.provider.metadata,
	};
}
