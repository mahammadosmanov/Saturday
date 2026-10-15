import {
	LayoutDashboard,
	Mic,
	Pause,
	Play,
} from "lucide-react";
import type { VoiceLanguage } from "../domain/voice";
import { voiceCopy } from "../domain/voice-copy";
import type { CommandItem } from "../components/CommandPalette";
import {
	BUILTIN_MODULE_API_VERSION,
	BuiltInModuleRegistry,
	type BuiltInModule,
} from "./module-registry";

export interface BuiltInCommandContext {
	language: VoiceLanguage;
	motionEnabled: boolean;
	actions: {
		closePanels(): void;
		openConversation(): void;
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
		activate: ({ actions, language }) => ({
			contributions: [
				{
					id: "overview",
					label: voiceCopy(language, "ui.return_to_core"),
					description: voiceCopy(language, "ui.close_secondary_panels"),
					keywords: "home overview focus ana çekirdek dön",
					icon: LayoutDashboard,
					run: actions.closePanels,
				},
			],
		}),
	},
	{
		manifest: manifest("core.voice", "Native voice conversation"),
		activate: ({ actions, language }) => ({
			contributions: [
				{
					id: "voice",
					label: voiceCopy(language, "ui.open_voice"),
					description: voiceCopy(language, "ui.open_voice_description"),
					keywords: "voice microphone conversation speak audio listen ses mikrofon konuşma",
					icon: Mic,
					run: actions.openConversation,
				},
			],
		}),
	},
	{
		manifest: manifest("core.motion", "Motion control"),
		activate: ({ actions, motionEnabled, language }) => ({
			contributions: [
				{
					id: "motion",
					label: voiceCopy(language, motionEnabled ? "ui.pause_motion" : "ui.resume_motion"),
					description: voiceCopy(language, "ui.motion_description"),
					keywords: "stop motion accessibility duraklat animasyon hareket",
					icon: motionEnabled ? Pause : Play,
					run: actions.toggleMotion,
				},
			],
		}),
	},
];

export function buildBuiltInCommands(context: BuiltInCommandContext): BuiltInCommandResult {
	const registry = new BuiltInModuleRegistry<BuiltInCommandContext, CommandItem>();
	for (const module of modules) registry.register(module);
	registry.activateAll(context);
	return { commands: registry.getContributions() };
}
