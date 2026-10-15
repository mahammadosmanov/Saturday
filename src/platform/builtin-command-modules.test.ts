import { describe, expect, it, vi } from "vitest";
import { buildBuiltInCommands } from "./builtin-command-modules";

function context(language: "en" | "tr" = "en") {
	return {
		language,
		motionEnabled: true,
		actions: {
			closePanels: vi.fn(),
			openConversation: vi.fn(),
			toggleMotion: vi.fn(),
		},
	};
}

describe("built-in command modules", () => {
	it("exposes only commands backed by native or real UI effects", () => {
		const result = buildBuiltInCommands(context());
		expect(result.commands.map((command) => command.id)).toEqual([
			"overview",
			"voice",
			"motion",
		]);
	});

	it("opens the native conversation surface", () => {
		const ctx = context("en");
		const voiceCommand = buildBuiltInCommands(ctx).commands.find((command) => command.id === "voice");
		voiceCommand?.run();
		expect(ctx.actions.openConversation).toHaveBeenCalledOnce();
	});

});
