import {
	ArrowRight,
	CalendarDays,
	Command,
	LayoutDashboard,
	Mic,
	Settings2,
} from "lucide-react";
import { MotionConfig } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { CommandPalette } from "./components/CommandPalette";
import { Conversation } from "./components/Conversation";
import { Core } from "./components/Core";
import { SettingsDialog } from "./components/SettingsDialog";
import { SocialStudio } from "./components/SocialStudio";
import {
	WorkspaceDialog,
	type WorkspaceTab,
} from "./components/WorkspaceDialog";
import { useConversation } from "./hooks/useConversation";
import { useMicrophone } from "./hooks/useMicrophone";
import { useActivity, useDiagnostic } from "./hooks/useSimulation";
import { useMotionEnabled, useWorkspace } from "./hooks/useWorkspace";
import { buildBuiltInCommands } from "./platform/builtin-command-modules";
import "./styles.css";
import "./dialogs.css";
import "./instruments.css";
import "./conversation.css";
import "./minimal.css";
import "./social-studio.css";

type Panel =
	| "workspace"
	| "settings"
	| "palette"
	| "conversation"
	| "social"
	| null;
export default function App() {
	const [panel, setPanel] = useState<Panel>(null);
	const [tab, setTab] = useState<WorkspaceTab>("tasks");
	const [draft, setDraft] = useState("");
	const input = useRef<HTMLInputElement>(null);
	const { workspace, updateWorkspace, storageWarning, reset } = useWorkspace();
	const enabled = useMotionEnabled(workspace.motion);
	const { entries, log } = useActivity();
	const { scan, start, cancel } = useDiagnostic(log);
	const conversation = useConversation((action) => {
		if (action === "diagnostic") {
			setTab("diagnostics");
			start();
		} else setPanel(null);
	});
	const microphone = useMicrophone();
	const stopMicrophone = microphone.stop;
	const microphoneActive =
		microphone.status === "listening" || microphone.status === "requesting";
	const microphoneLabel =
		microphone.status === "requesting"
			? "Cancel microphone request"
			: microphoneActive
				? "Stop listening"
				: "Start listening";
	useEffect(() => {
		if (panel !== null) stopMicrophone();
	}, [panel, stopMicrophone]);
	const cancelConversation = conversation.cancel;
	useEffect(() => {
		if (panel !== "conversation") cancelConversation();
	}, [panel, cancelConversation]);
	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			// An exiting Radix layer can still receive Escape during a swap.
			// The app owns the active panel, not the retiring animation layer.
			if (event.key === "Escape") {
				stopMicrophone();
				setPanel(null);
				return;
			}
			if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
				event.preventDefault();
				if (panel === null || panel === "palette")
					setPanel((value) => (value === "palette" ? null : "palette"));
			}
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [panel, stopMicrophone]);
	const openTools = (next: WorkspaceTab) => {
		setTab(next);
		setPanel("workspace");
	};
	const toggleMotion = () => {
		updateWorkspace((w) => ({ ...w, motion: !w.motion }));
		log(
			"Motion preference changed",
			workspace.motion
				? "Ambient motion paused."
				: "Ambient motion resumed; system preference takes priority.",
		);
	};
	const voice = () => {
		setPanel(null);
		conversation.cancel();
		if (microphoneActive) microphone.stop();
		else void microphone.start(workspace.microphoneDeviceId);
	};
	const { commands, modules } = buildBuiltInCommands({
		microphoneLabel,
		diagnosticRunning: scan.status === "running",
		motionEnabled: workspace.motion,
		actions: {
			closePanels: () => setPanel(null),
			openConversation: () => setPanel("conversation"),
			openWorkspace: openTools,
			toggleDiagnostic: () => {
				if (scan.status === "running") cancel();
				else start();
			},
			toggleVoice: voice,
			toggleMotion,
			openSettings: () => setPanel("settings"),
		},
	});
	const close = (name: Exclude<Panel, null>) => (open: boolean) =>
		setPanel((current) => (open ? name : current === name ? null : current));
	return (
		<MotionConfig
			reducedMotion="user"
			transition={{ duration: enabled ? 0.2 : 0 }}
		>
			<div
				className="app minimal-app"
				data-accent={workspace.accent}
				data-motion={enabled ? "on" : "off"}
				onClickCapture={(event) => {
					if (
						event.button !== 0 ||
						event.detail === 0 ||
						!(event.target instanceof Element)
					)
						return;
					const button = event.target.closest("button");
					if (button && !button.disabled) button.focus({ preventScroll: true });
				}}
			>
				<header className="minimal-header">
					<a href="#main" className="skip-link">
						Skip to main content
					</a>
					<div className="minimal-actions">
						<button
							type="button"
							className="button social-trigger"
							onClick={() => setPanel("social")}
						>
							<CalendarDays size={15} />
							<span>Social Studio</span>
						</button>
						<button
							type="button"
							className="button workspace-trigger"
							onClick={() => openTools(tab)}
							aria-label="Workspace"
						>
							<LayoutDashboard size={15} />
							<span>Workspace</span>
						</button>
						<button
							type="button"
							className="button icon-button"
							aria-label="Settings"
							title="Settings"
							onClick={() => setPanel("settings")}
						>
							<Settings2 size={16} />
						</button>
					</div>
				</header>
				<main id="main" className="minimal-main">
					{storageWarning && (
						<p className="storage-warning" role="status">
							{storageWarning}
						</p>
					)}
					<Core
						enabled={enabled}
						accent={workspace.accent}
						audio={microphone.signal}
						notice={
							microphone.error ||
							(microphone.status === "requesting"
								? "Waiting for microphone permission…"
								: undefined)
						}
						state={
							microphone.status === "listening"
								? "listening"
								: conversation.phase !== "standby"
									? conversation.phase
									: scan.status === "running"
										? "scanning"
										: "standby"
						}
					/>
					<div className="composer-wrap">
						<form
							className="core-composer"
							onSubmit={(event) => {
								event.preventDefault();
								if (!draft.trim() || conversation.pending) return;
								if (conversation.send(draft)) {
									setDraft("");
									setPanel("conversation");
								}
							}}
						>
							<button
								id="command-trigger"
								className="composer-command"
								type="button"
								aria-label="Open command palette"
								title="Commands (⌘K)"
								onClick={() => setPanel("palette")}
							>
								<Command size={16} />
							</button>
							<input
								ref={input}
								aria-label="Command message"
								maxLength={500}
								placeholder="What can I do for you?"
								value={draft}
								onChange={(event) => setDraft(event.target.value)}
								autoComplete="off"
							/>
							<button
								type="button"
								className="composer-voice"
								aria-label={microphoneLabel}
								aria-pressed={microphoneActive}
								title={`${microphoneLabel} · local audio only`}
								onClick={voice}
							>
								<Mic size={17} />
							</button>
							<button
								type="submit"
								className="composer-send"
								aria-label="Send command"
								disabled={!draft.trim() || conversation.pending}
							>
								<ArrowRight size={19} />
							</button>
						</form>
						<div className="composer-foot">
							<span>Local preview · no AI connected</span>
							<button
								type="button"
								onClick={() => setPanel("conversation")}
								aria-label="Open conversation"
							>
								Conversation <span aria-hidden="true">↗</span>
							</button>
						</div>
					</div>
				</main>
				<div id="overlay-root" />
				<WorkspaceDialog
					open={panel === "workspace"}
					onOpenChange={close("workspace")}
					tab={tab}
					onTab={setTab}
					workspace={workspace}
					onUpdate={updateWorkspace}
					scan={scan}
					onStart={start}
					onCancel={cancel}
					entries={entries}
				/>
				<CommandPalette
					open={panel === "palette"}
					onOpenChange={close("palette")}
					commands={commands}
					animate={enabled}
				/>
				<Conversation
					open={panel === "conversation"}
					onOpenChange={close("conversation")}
					messages={conversation.messages}
					pending={conversation.pending}
					onSend={conversation.send}
					onVoice={voice}
					animate={enabled}
				/>
				<SettingsDialog
					open={panel === "settings"}
					onOpenChange={close("settings")}
					workspace={workspace}
					modules={modules}
					onUpdate={updateWorkspace}
					onReset={reset}
				/>
				<SocialStudio
					open={panel === "social"}
					onClose={() => setPanel(null)}
				/>
			</div>
		</MotionConfig>
	);
}
