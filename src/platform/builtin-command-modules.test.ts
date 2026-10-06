import { describe, expect, it, vi } from "vitest";
import { buildBuiltInCommands } from "./builtin-command-modules";

function context() {
	return {
		microphoneLabel: "Start listening",
		motionEnabled: true,
		actions: {
			closePanels: vi.fn(),
			openConversation: vi.fn(),
			toggleVoice: vi.fn(),
			toggleMotion: vi.fn(),
		},
	};
}

describe("built-in command modules", () => {
	it("exposes only commands backed by retained UI surfaces", () => {
		const result = buildBuiltInCommands(context());

		expect(result.commands.map((command) => command.id)).toEqual([
			"overview",
			"conversation",
			"voice",
			"motion",
		]);
	});

	it("keeps voice and motion labels responsive to current state", () => {
		const ctx = context();
		ctx.microphoneLabel = "Stop listening";
		ctx.motionEnabled = false;

		const { commands } = buildBuiltInCommands(ctx);

		expect(commands.find((command) => command.id === "voice")?.label).toBe(
			"Stop listening",
		);
		expect(commands.find((command) => command.id === "motion")?.label).toBe(
			"Resume animations",
		);
	});
});
