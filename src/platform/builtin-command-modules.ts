import {
	LayoutDashboard,
	Mic,
	Pause,
	Play,
	Terminal,
} from "lucide-react";
import type { CommandItem } from "../components/CommandPalette";
import {
	BUILTIN_MODULE_API_VERSION,
	BuiltInModuleRegistry,
	type BuiltInModule,
} from "./module-registry";

export interface BuiltInCommandContext {
	microphoneLabel: string;
	motionEnabled: boolean;
	actions: {
		closePanels(): void;
		openConversation(): void;
		toggleVoice(): void;
		toggleMotion(): void;
	};
}

export interface BuiltInCommandResult {
	commands: CommandItem[];
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
					label: "Talk to S.A.T.U.R.D.A.Y",
					description: "Open your session conversation",
					keywords: "chat assistant text history",
					icon: Terminal,
					run: actions.openConversation,
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
		manifest: manifest("core.motion", "Motion control"),
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
	return { commands: registry.getContributions() };
}
