import { ArrowRight, Command, Mic } from "lucide-react";
import { MotionConfig } from "motion/react";
import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { CommandPalette } from "./components/CommandPalette";
import { Conversation } from "./components/Conversation";
import { Core } from "./components/Core";
import { voiceCopy, voiceErrorCopyKey, voiceNoticeCopyKey, voicePhaseCopyKey } from "./domain/voice-copy";
import { validateTypedText, type VoiceLanguage } from "./domain/voice";
import { useVoiceSession } from "./hooks/useVoiceSession";
import { useMotionEnabled, useWorkspace } from "./hooks/useWorkspace";
import { buildBuiltInCommands } from "./platform/builtin-command-modules";
import "./styles.css";
import "./dialogs.css";
import "./conversation.css";
import "./minimal.css";

const encoder = new TextEncoder();
type Panel = "palette" | "conversation" | null;

export default function App() {
	const [panel, setPanel] = useState<Panel>(null);
	const [draft, setDraft] = useState("");
	const [language, setLanguage] = useState<VoiceLanguage>("en");
	const pendingMainSubmission = useRef<{ text: string } | null>(null);
	const { workspace, updateWorkspace, storageWarning } = useWorkspace();
	const enabled = useMotionEnabled(workspace.motion);
	const closeAfterCleanup = useCallback(() => {
		pendingMainSubmission.current = null;
		setPanel((current) => current === "conversation" ? null : current);
	}, []);
	const session = useVoiceSession(panel === "conversation", closeAfterCleanup);
	const snapshot = session.state?.snapshot;
	const phase = snapshot?.metadata.phase ?? (session.supported ? "setup_required" : "unavailable");
	useEffect(() => {
		const clearPending = () => {
			if (document.visibilityState === "hidden") pendingMainSubmission.current = null;
		};
		window.addEventListener("pagehide", clearPending);
		document.addEventListener("visibilitychange", clearPending);
		return () => {
			window.removeEventListener("pagehide", clearPending);
			document.removeEventListener("visibilitychange", clearPending);
		};
	}, []);
	useEffect(() => {
		if (panel !== "conversation") {
			pendingMainSubmission.current = null;
			return;
		}
		const pending = pendingMainSubmission.current;
		if (!pending) return;
		if (session.status === "failed" || session.status === "unsupported") {
			pendingMainSubmission.current = null;
			return;
		}
		if (session.status !== "ready" || !snapshot) return;
		pendingMainSubmission.current = null;
		setDraft("");
		void session.submitText(pending.text, snapshot.metadata.preferences.language_mode ?? "auto");
	}, [panel, session.status, snapshot, session.submitText]);
	const typedValidation = validateTypedText(draft);

	let notice = voiceCopy(language, voicePhaseCopyKey(phase));
	if (session.status === "connecting") notice = voiceCopy(language, "voice.connecting");
	else if (session.status === "unsupported") notice = voiceCopy(language, "voice.browser_unavailable");
	else if (session.error === "cleanup") notice = voiceCopy(language, "voice.cleanup_failed");
	else if (session.error !== null) notice = voiceCopy(language, "voice.error");
	else if (snapshot?.metadata.error) notice = voiceCopy(language, voiceErrorCopyKey(snapshot.metadata.error));
	else if (snapshot?.metadata.notices.length) {
		const code = snapshot.metadata.notices[snapshot.metadata.notices.length - 1];
		if (code) notice = voiceCopy(language, voiceNoticeCopyKey(code));
	}

	const storageWarningKey = storageWarning
		? storageWarning.includes("unreadable")
			? "ui.saved_preferences_unreadable"
			: "ui.no_storage"
		: null;

	const openConversation = () => setPanel("conversation");
	const closeConversation = () => {
		pendingMainSubmission.current = null;
		void session.closeSurface("surface_closed").then((closed) => {
			if (closed) setPanel((current) => current === "conversation" ? null : current);
		});
	};
	const openPalette = useCallback(() => {
		if (panel === "palette") {
			setPanel(null);
			return;
		}
		if (panel === "conversation") {
			pendingMainSubmission.current = null;

			void session.closeSurface("module_disabled").then((closed) => {
				if (closed) setPanel("palette");
			});
			return;
		}
		setPanel("palette");
	}, [panel, session.closeSurface]);
	useEffect(() => {
		const focusClickedButton = (event: MouseEvent) => {
			if (event.button !== 0 || event.detail === 0 || !(event.target instanceof Element)) return;
			const button = event.target.closest("button");
			if (button && !button.disabled) button.focus({ preventScroll: true });
		};
		document.addEventListener("click", focusClickedButton, true);
		return () => document.removeEventListener("click", focusClickedButton, true);
	}, []);
	useEffect(() => {
		const handleShortcut = (event: KeyboardEvent) => {
			if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
				event.preventDefault();
				openPalette();
			}
		};
		document.addEventListener("keydown", handleShortcut, true);
		return () => document.removeEventListener("keydown", handleShortcut, true);
	}, [openPalette]);
	const closePalette = (open: boolean) => {
		if (!open) setPanel((current) => current === "palette" ? null : current);
	};
	const submitMainRequest = (event: FormEvent<HTMLFormElement>) => {
		event.preventDefault();
		if (!session.supported || !typedValidation.ok || session.busy || pendingMainSubmission.current) return;
		pendingMainSubmission.current = { text: draft };
		setPanel("conversation");
	};
	const toggleMotion = () => updateWorkspace((current) => ({ ...current, motion: !current.motion }));
	const commands = buildBuiltInCommands({
		language,
		motionEnabled: workspace.motion,
		actions: {
			closePanels: () => setPanel(null),
			openConversation,
			toggleMotion,
		},
	}).commands;

	return (
		<MotionConfig reducedMotion="user" transition={{ duration: enabled ? 0.2 : 0 }}>
			<div
				className="app minimal-app"
				data-accent={workspace.accent}
				data-motion={enabled ? "on" : "off"}
			>
				<header className="minimal-header">
					<a href="#main" className="skip-link">{voiceCopy(language, "ui.skip_main")}</a>
					<strong className="app-brand">{voiceCopy(language, "ui.app_title")}</strong>
					<div className="header-actions">
						<button
							type="button"
							className="button language-toggle"
							aria-label={voiceCopy(language, language === "en" ? "ui.switch_to_turkish" : "ui.switch_to_english")}
							onClick={() => setLanguage(language === "en" ? "tr" : "en")}
						>
							{language === "en" ? "TR" : "EN"}
						</button>
						<button type="button" className="button" aria-label={voiceCopy(language, "ui.open_palette")} onClick={openPalette}>
							<Command size={16} aria-hidden="true" />
							<span>{voiceCopy(language, "ui.command_palette")}</span>
							<kbd>⌘K</kbd>
						</button>
					</div>
				</header>
				<main id="main" className="minimal-main">
					{storageWarningKey && <p className="storage-warning" role="status">{voiceCopy(language, storageWarningKey)}</p>}
					<Core
						enabled={enabled}
						phase={phase}
						language={language}
						notice={notice}
					/>
					<div className="composer-wrap">
						<form className="core-composer" onSubmit={submitMainRequest}>
							<button id="command-trigger" className="composer-command" type="button" aria-label={voiceCopy(language, "ui.open_palette")} title={voiceCopy(language, "ui.open_palette")} onClick={openPalette}>
								<Command size={16} aria-hidden="true" />
							</button>
							<label htmlFor="main-request" className="sr-only">{voiceCopy(language, "ui.typed_input")}</label>
							<input
								id="main-request"
								aria-label={voiceCopy(language, "ui.typed_input")}
								placeholder={voiceCopy(language, "ui.typed_placeholder")}
								value={draft}
								onChange={(event) => setDraft(event.currentTarget.value)}
								autoComplete="off"
								disabled={!session.supported || session.busy}
							/>
							<button type="button" className="composer-voice" aria-label={voiceCopy(language, "ui.open_voice")} title={voiceCopy(language, "ui.open_voice")} onClick={openConversation}>
								<Mic size={17} aria-hidden="true" />
							</button>
							<button type="submit" className="composer-send" aria-label={voiceCopy(language, "ui.send")} disabled={!session.supported || !typedValidation.ok || session.busy}>
								<ArrowRight size={19} aria-hidden="true" />
							</button>
						</form>
						<div className="composer-foot">
							<span>{session.supported ? voiceCopy(language, "voice.local_privacy") : voiceCopy(language, "voice.browser_typed_unavailable")}</span>
							<button type="button" onClick={openConversation} aria-label={voiceCopy(language, "ui.open_conversation")}>
								{voiceCopy(language, "ui.open_conversation")} <span aria-hidden="true">↗</span>
							</button>
						</div>
						{draft.trim() && !typedValidation.ok && <p role="alert" className="voice-inline-warning">{voiceCopy(language, "voice.typed_limit")}</p>}
						<p className="composer-counter">{Array.from(draft).length} / 500 · {voiceCopy(language, "voice.text_bytes")}: {encoder.encode(draft).byteLength} / 2,000</p>
					</div>
				</main>
				<div id="overlay-root" />
				<CommandPalette
					open={panel === "palette"}
					onOpenChange={closePalette}
					commands={commands}
					animate={enabled}
					language={language}
				/>
				<Conversation
					open={panel === "conversation"}
					onOpenChange={(open) => { if (open) openConversation(); else closeConversation(); }}
					session={session}
					language={language}
					animate={enabled}
				/>
			</div>
		</MotionConfig>
	);
}
