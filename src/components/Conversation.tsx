import { motion } from "motion/react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import {
	compareDecimalU64,
	validateReviewedTranscript,
	validateTypedText,
	type AppIntentDescriptor,
	type DecimalU64,
	type VoiceLanguage,
	type VoiceLiveText,
	type VoicePendingText,
} from "../domain/voice";
import {
	voiceCaptureLifecycleCopyKey,
	voiceCopy,
	voiceErrorCopyKey,
	voiceNoticeCopyKey,
	voicePhaseCopyKey,
} from "../domain/voice-copy";
import type { VoiceSessionController as Session } from "../hooks/useVoiceSession";
import { Emblem } from "./Emblem";
import { Modal } from "./Modal";
import { VoiceSetup } from "./VoiceSetup";

interface StreamText {
	text: string;
	language: VoiceLanguage | null;
	partial: boolean;
}

function collectStream(
	pending: VoicePendingText[],
	live: VoiceLiveText[],
	stream: "transcript" | "answer",
): StreamText {
	const chunks = new Map<DecimalU64, StreamText>();
	for (const item of pending) {
		if (item.stream === stream) {
			chunks.set(item.sequence, {
				text: item.text,
				language: item.language,
				partial: item.partial,
			});
		}
	}
	for (const item of live) {
		if (item.stream === stream) {
			chunks.set(item.sequence, {
				text: item.text,
				language: item.language,
				partial: item.partial,
			});
		}
	}
	const ordered = [...chunks.entries()].sort(([left], [right]) => compareDecimalU64(left, right));
	return {
		text: ordered.map(([, item]) => item.text).join(""),
		language: ordered.find(([, item]) => item.language !== null)?.[1].language ?? null,
		partial: ordered.some(([, item]) => item.partial),
	};
}

function intentCopyKey(intent: AppIntentDescriptor): Parameters<typeof voiceCopy>[1] {
	switch (intent.intent) {
		case "overview":
			return "voice.intent_overview";
		case "conversation":
			return "voice.intent_conversation";
		case "motion":
			return intent.enabled ? "voice.intent_motion_on" : "voice.intent_motion_off";
	}
}

function languageCopyKey(language: VoiceLanguage): "voice.language_en" | "voice.language_tr" {
	return language === "en" ? "voice.language_en" : "voice.language_tr";
}

export function Conversation({
	open,
	onOpenChange,
	session,
	language,
	animate,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	session: Session;
	language: VoiceLanguage;
	animate: boolean;
}) {
	const [draft, setDraft] = useState("");
	const [tab, setTab] = useState<"conversation" | "setup">("conversation");
	const [reviewDraft, setReviewDraft] = useState("");
	const [reviewLanguage, setReviewLanguage] = useState<VoiceLanguage | null>(null);
	const [confirmClear, setConfirmClear] = useState(false);
	const input = useRef<HTMLTextAreaElement>(null);
	const log = useRef<HTMLDivElement>(null);
	const snapshot = session.state?.snapshot;
	const metadata = snapshot?.metadata;
	const history = snapshot?.history ?? [];
	const pending = snapshot?.pending ?? [];
	const transcript = collectStream(pending, session.state?.transcript_deltas ?? [], "transcript");
	const answer = collectStream(pending, session.state?.answer_deltas ?? [], "answer");
	const phase = metadata?.phase ?? "unavailable";
	const owner = snapshot?.owner ?? null;
	const lastMessageId = history.at(-1)?.id;
	const captureElapsed = session.state?.meter?.elapsed_ms ?? metadata?.capture_elapsed_ms ?? 0;
	const captureLevel = session.state?.meter?.level ?? 0;
	const typedValidation = validateTypedText(draft);
	const transcriptValidation = validateReviewedTranscript(reviewDraft);
	const hasTranscript = transcript.text.trim().length > 0;
	const reviewingTranscript = phase === "needs_review" && hasTranscript;
	const staleInput = Boolean(
		metadata?.preferences.input_device_handle &&
		!snapshot?.input_devices.some((device) => device.handle === metadata.preferences.input_device_handle),
	);
	const captureReadiness = metadata?.readiness.capture;
	const canRequestCapture = Boolean(
		captureReadiness &&
		(captureReadiness.state === "ready" || captureReadiness.code === "permission_required") &&
		!staleInput,
	);
	const canCancel = owner !== null || metadata?.cancel_available === true;

	useEffect(() => {
		if (open && log.current && lastMessageId)
			log.current.scrollTop = log.current.scrollHeight;
	}, [open, lastMessageId]);

	useEffect(() => {
		setReviewDraft(transcript.text);
		setReviewLanguage(null);
	}, [transcript.text]);

	const submitTyped = async (event: FormEvent<HTMLFormElement>) => {
		event.preventDefault();
		if (!session.supported || !typedValidation.ok || session.busy) return;
		const result = await session.submitText(draft, metadata?.preferences.language_mode ?? "auto");
		if (result?.accepted) setDraft("");
	};

	const startCapture = () => {
		if (!snapshot || !canRequestCapture || session.busy) return;
		const preferences = snapshot.metadata.preferences;
		const device = preferences.input_device_handle
			? snapshot.input_devices.find((candidate) => candidate.handle === preferences.input_device_handle) ?? null
			: null;
		if (preferences.input_device_handle && !device) return;
		void session.start(preferences.language_mode ?? "auto", device);
	};

	const sendReviewedTranscript = async (event: FormEvent<HTMLFormElement>) => {
		event.preventDefault();
		if (!owner || reviewLanguage === null || !transcriptValidation.ok || session.busy) return;
		const result = await session.confirmTranscript(reviewDraft, reviewLanguage);
		if (result?.accepted) {
			setReviewLanguage(null);
			setReviewDraft("");
		}
	};

	const currentNotice = metadata?.notices[metadata.notices.length - 1];
	let statusMessage = voiceCopy(language, voicePhaseCopyKey(phase));
	if (session.status === "connecting") statusMessage = voiceCopy(language, "voice.connecting");
	else if (session.status === "unsupported") statusMessage = voiceCopy(language, "voice.browser_unavailable");
	else if (session.error === "cleanup") statusMessage = voiceCopy(language, "voice.cleanup_failed");
	else if (session.error !== null) statusMessage = voiceCopy(language, "voice.error");
	else if (metadata?.error) statusMessage = voiceCopy(language, voiceErrorCopyKey(metadata.error));
	else if (session.state?.capture_lifecycle) statusMessage = voiceCopy(language, voiceCaptureLifecycleCopyKey(session.state.capture_lifecycle));
	else if (currentNotice) statusMessage = voiceCopy(language, voiceNoticeCopyKey(currentNotice));

	return (
		<Modal
			open={open}
			onOpenChange={onOpenChange}
			title={voiceCopy(language, "voice.title")}
			description={voiceCopy(language, "voice.description")}
			className="conversation-dialog"
			initialFocus={session.supported ? input : undefined}
			language={language}
		>
			<div className="conversation-identity">
				<Emblem small />
				<span>
					S.A.T.U.R.D.A.Y<strong>{voiceCopy(language, "voice.native_status")}</strong>
				</span>
				<span className="conversation-badge" data-status={session.status}>
					<i className="status-dot" aria-hidden="true" />
					{session.status === "ready" ? voiceCopy(language, "voice.local_assistant") : voiceCopy(language, "voice.not_connected")}
				</span>
			</div>
			<div className="voice-tabs" role="tablist" aria-label={voiceCopy(language, "voice.title")}>
				<button type="button" role="tab" aria-selected={tab === "conversation"} onClick={() => setTab("conversation")}>
					{voiceCopy(language, "voice.conversation_tab")}
				</button>
				<button type="button" role="tab" aria-selected={tab === "setup"} onClick={() => setTab("setup")}>
					{voiceCopy(language, "voice.setup_tab")}
				</button>
			</div>
			{tab === "setup" ? (
				<VoiceSetup language={language} session={session} active={open} />
			) : (
				<div className="conversation-content">
					<div className="conversation-status" role="status" aria-live="polite">
						<p>{statusMessage}</p>
						{metadata && (
							<div className="conversation-capture-status">
								<span>{voiceCopy(language, "voice.capture_elapsed")}: {Math.min(30, Math.floor(captureElapsed / 1000))}/30 {voiceCopy(language, "voice.seconds")}</span>
								<meter min={0} max={1} value={captureLevel} aria-label={voiceCopy(language, "voice.capture_level")} />
							</div>
						)}
					</div>
					<div className="voice-control-row">
						<button type="button" className="button primary" disabled={!canRequestCapture || session.busy || owner !== null} onClick={startCapture}>
							{voiceCopy(language, "voice.start")}
						</button>
						<button type="button" className="button" disabled={phase !== "listening" || session.busy} onClick={() => void session.finish()}>
							{voiceCopy(language, "voice.end_capture")}
						</button>
						<button type="button" className="button cancel-button" disabled={!canCancel} onClick={() => void session.cancelTurn()}>
							{voiceCopy(language, "voice.cancel")}
						</button>
					</div>
					{staleInput && <p className="voice-inline-warning" role="alert">{voiceCopy(language, "voice.reselect_device")}</p>}
					<p className="voice-prestart-disclosure">{voiceCopy(language, "voice.microphone_disclosure")}</p>
					<div className="conversation-messages" ref={log} tabIndex={open ? 0 : undefined} role="log" aria-label={voiceCopy(language, "voice.title")} aria-live="off" aria-relevant="additions text">
						{history.length === 0 && !transcript.text && !answer.text && <p className="voice-empty-history">{voiceCopy(language, "voice.empty_history")}</p>}
						{history.map((message) => (
							<motion.article
								className={`message ${message.role}`}
								key={message.id}
								initial={animate ? { opacity: 0, transform: "translateY(6px)" } : false}
								animate={{ opacity: 1, transform: "translateY(0px)" }}
								transition={{ duration: animate ? 0.22 : 0 }}
							>
								<span className="message-label">
									{voiceCopy(language, message.role === "assistant" ? "voice.role_assistant" : "voice.role_you")}
								</span>
								<span className="message-meta">{voiceCopy(language, languageCopyKey(message.language))} · {voiceCopy(language, message.source === "speech" ? "voice.source_speech" : "voice.source_typed")}</span>
								<p>{message.text}</p>
							</motion.article>
						))}
						{transcript.text && !reviewingTranscript && (
							<article className="message user live-message">
								<span className="message-label">{voiceCopy(language, "voice.transcript")}</span>
								{transcript.language && <span className="message-meta">{voiceCopy(language, languageCopyKey(transcript.language))}</span>}
								<p>{transcript.text}</p>
							</article>
						)}
						{answer.text && (
							<article className="message assistant live-message">
								<span className="message-label">{voiceCopy(language, "voice.answer")}</span>
								{answer.language && <span className="message-meta">{voiceCopy(language, languageCopyKey(answer.language))}</span>}
								<p>{answer.text}{answer.partial ? ` · ${voiceCopy(language, "voice.partial_answer")}` : ""}</p>
							</article>
						)}
					</div>
					{reviewingTranscript && (
						<form className="transcript-review" onSubmit={(event) => void sendReviewedTranscript(event)}>
							<label htmlFor="voice-transcript-review">{voiceCopy(language, "voice.review_text_label")}</label>
							<textarea id="voice-transcript-review" value={reviewDraft} onChange={(event) => setReviewDraft(event.currentTarget.value)} rows={4} />
							<p>{voiceCopy(language, "voice.review_not_edited")}</p>
							<label htmlFor="voice-transcript-language">{voiceCopy(language, "voice.choose_transcript_language")}</label>
							<select id="voice-transcript-language" value={reviewLanguage ?? ""} onChange={(event) => setReviewLanguage(event.currentTarget.value === "en" || event.currentTarget.value === "tr" ? event.currentTarget.value : null)}>
								<option value="">{voiceCopy(language, "voice.transcript_language_required")}</option>
								<option value="en">{voiceCopy(language, "voice.language_en")}</option>
								<option value="tr">{voiceCopy(language, "voice.language_tr")}</option>
							</select>
							{reviewDraft && !transcriptValidation.ok && <p role="alert">{voiceCopy(language, transcriptValidation.reason === "invalid_unicode" ? "voice.invalid_unicode" : "voice.transcript_limit")}</p>}
							<button type="submit" className="button primary" disabled={!owner || reviewLanguage === null || !transcriptValidation.ok || session.busy}>
								{voiceCopy(language, "voice.confirm_transcript")}
							</button>
						</form>
					)}
					{session.state?.intents.map((intent) => (
						<div className="voice-intent" key={intent.intent_id}>
							<p>{voiceCopy(language, "voice.intent_prompt")}</p>
							<strong>{voiceCopy(language, intentCopyKey(intent))}</strong>
							<div className="voice-control-row">
								<button type="button" className="button primary" disabled={session.busy} onClick={() => void session.acknowledgeIntent(intent.owner, intent.intent_id, true)}>{voiceCopy(language, "voice.accept_intent")}</button>
								<button type="button" className="button" disabled={session.busy} onClick={() => void session.acknowledgeIntent(intent.owner, intent.intent_id, false)}>{voiceCopy(language, "voice.reject_intent")}</button>
							</div>
						</div>
					))}
					<form className="conversation-composer" onSubmit={(event) => void submitTyped(event)}>
						<label htmlFor="voice-typed-request" className="sr-only">{voiceCopy(language, "voice.type_placeholder")}</label>
						<textarea
							id="voice-typed-request"
							ref={input}
							aria-label={voiceCopy(language, "voice.type_placeholder")}
							placeholder={voiceCopy(language, "voice.type_placeholder")}
							value={draft}
							onChange={(event) => setDraft(event.currentTarget.value)}
							rows={2}
							disabled={!session.supported || session.busy}
						/>
						<button type="submit" className="send-message" aria-label={voiceCopy(language, "voice.submit_text")} disabled={!session.supported || !typedValidation.ok || session.busy}>
							{voiceCopy(language, "voice.submit_text")}
						</button>
					</form>
					<div className="conversation-foot">
						<span>{draft.length} / 500 · {voiceCopy(language, "voice.text_bytes")}: {new TextEncoder().encode(draft).byteLength} / 2,000</span>
						<span>{voiceCopy(language, "voice.local_privacy")}</span>
					</div>
					{draft.trim() && !typedValidation.ok && <p role="alert" className="voice-inline-warning">{voiceCopy(language, "voice.typed_limit")}</p>}
					<div className="conversation-footer-actions">
						<button type="button" disabled={session.busy} onClick={() => setConfirmClear(true)}>{voiceCopy(language, "voice.confirm_clear")}</button>
					</div>
					{confirmClear && (
						<div className="voice-clear-confirm" role="alertdialog" aria-modal="false" aria-label={voiceCopy(language, "voice.clear_confirm")}>
							<p>{voiceCopy(language, "voice.clear_confirm")}</p>
							<button type="button" className="button primary" disabled={session.busy} onClick={async () => { const result = await session.clearSession(); if (result?.accepted) setConfirmClear(false); }}>{voiceCopy(language, "voice.confirm_clear")}</button>
							<button type="button" className="button" onClick={() => setConfirmClear(false)}>{voiceCopy(language, "voice.close")}</button>
						</div>
					)}
				</div>
			)}
		</Modal>
	);
}
