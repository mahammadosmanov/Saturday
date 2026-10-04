import {
	Activity,
	LayoutDashboard,
	Mic,
	Pause,
	Play,
	ScanLine,
	Settings2,
	Terminal,
} from "lucide-react";
import type { CommandItem } from "../components/CommandPalette";
import type { WorkspaceTab } from "../components/WorkspaceDialog";
import {
	BUILTIN_MODULE_API_VERSION,
	BuiltInModuleRegistry,
	type BuiltInModule,
	type BuiltInModuleStatus,
} from "./module-registry";

export interface BuiltInCommandContext {
	microphoneLabel: string;
	diagnosticRunning: boolean;
	motionEnabled: boolean;
	actions: {
		closePanels(): void;
		openConversation(): void;
		openWorkspace(tab: WorkspaceTab): void;
		toggleDiagnostic(): void;
		toggleVoice(): void;
		toggleMotion(): void;
		openSettings(): void;
	};
}

export interface BuiltInCommandResult {
	commands: CommandItem[];
	modules: BuiltInModuleStatus[];
}

function manifest(id: string, name: string) {
	return {
		id,
		name,
		version: "1.0.0",
		apiVersion: BUILTIN_MODULE_API_VERSION,
	} as const;
}

const modules: readonly BuiltInModule<
	BuiltInCommandContext,
	CommandItem
>[] = [
	{
		manifest: manifest("core.navigation", "Core navigation"),
		activate: ({ actions }) => ({
			contributions: [
				{
					id: "overview",
					label: "Return to core",
					description: "Close secondary panels",
					keywords: "home overview focus",
					icon: LayoutDashboard,
					run: actions.closePanels,
				},
				{
					id: "conversation",
					label: "Talk to J.A.R.V.I.S",
					description: "Open your session conversation",
					keywords: "chat assistant text history",
					icon: Terminal,
					run: actions.openConversation,
				},
			],
		}),
	},
	{
		manifest: manifest("core.workspace", "Workspace"),
		activate: ({ actions, diagnosticRunning }) => ({
			contributions: [
				{
					id: "tasks",
					label: "Open saved tasks",
					description: "Your local objectives and checklist",
					keywords: "objectives workspace tasks",
					icon: LayoutDashboard,
					run: () => actions.openWorkspace("tasks"),
				},
				{
					id: "diagnostic",
					label: diagnosticRunning
						? "Cancel diagnostic"
						: "Run diagnostic simulation",
					description: "A scripted check, not hardware access",
					keywords: "scan check test",
					icon: ScanLine,
					run: () => {
						actions.toggleDiagnostic();
						actions.openWorkspace("diagnostics");
					},
				},
				{
					id: "systems",
					label: "Open systems diagnostics",
					description: "Inspect the demo diagnostic workflow",
					keywords: "systems",
					icon: ScanLine,
					run: () => actions.openWorkspace("diagnostics"),
				},
				{
					id: "activity",
					label: "View activity stream",
					description: "Local events from this session",
					keywords: "history events log",
					icon: Activity,
					run: () => actions.openWorkspace("activity"),
				},
			],
		}),
	},
	{
		manifest: manifest("core.voice", "Local voice visualization"),
		activate: ({ actions, microphoneLabel }) => ({
			contributions: [
				{
					id: "voice",
					label: microphoneLabel,
					description: "On-device sound visualization; no recording",
					keywords: "mic microphone speak audio listen",
					icon: Mic,
					run: actions.toggleVoice,
				},
			],
		}),
	},
	{
		manifest: manifest("core.appearance", "Appearance and settings"),
		activate: ({ actions, motionEnabled }) => ({
			contributions: [
				{
					id: "motion",
					label: motionEnabled ? "Pause animations" : "Resume animations",
					description: "Pause or resume from the current position",
					keywords: "stop motion accessibility",
					icon: motionEnabled ? Pause : Play,
					run: actions.toggleMotion,
				},
				{
					id: "settings",
					label: "Interface settings",
					description: "Theme, motion and reset controls",
					keywords: "preferences customize amber cyan",
					icon: Settings2,
					run: actions.openSettings,
				},
			],
		}),
	},
];

export function buildBuiltInCommands(
	context: BuiltInCommandContext,
): BuiltInCommandResult {
	const registry = new BuiltInModuleRegistry<
		BuiltInCommandContext,
		CommandItem
	>();
	for (const module of modules) registry.register(module);
	registry.activateAll(context);
	return {
		commands: registry.getContributions(),
		modules: registry.list(),
	};
}
