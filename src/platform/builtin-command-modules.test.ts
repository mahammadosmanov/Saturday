import { describe, expect, it, vi } from "vitest";
import { buildBuiltInCommands } from "./builtin-command-modules";

function context() {
	return {
		microphoneLabel: "Start listening",
		diagnosticRunning: false,
		motionEnabled: true,
		actions: {
			closePanels: vi.fn(),
			openConversation: vi.fn(),
			openWorkspace: vi.fn(),
			toggleDiagnostic: vi.fn(),
			toggleVoice: vi.fn(),
			toggleMotion: vi.fn(),
			openSettings: vi.fn(),
		},
	};
}

describe("built-in command modules", () => {
	it("publishes every existing command through named active modules", () => {
		const result = buildBuiltInCommands(context());

		expect(result.commands.map((command) => command.id)).toEqual([
			"overview",
			"conversation",
			"tasks",
			"diagnostic",
			"systems",
			"activity",
			"voice",
			"motion",
			"settings",
		]);
		expect(result.modules).toEqual([
			expect.objectContaining({ id: "core.navigation", state: "active" }),
			expect.objectContaining({ id: "core.workspace", state: "active" }),
			expect.objectContaining({ id: "core.voice", state: "active" }),
			expect.objectContaining({ id: "core.appearance", state: "active" }),
		]);
	});

	it("routes workspace and diagnostic commands through injected actions", () => {
		const ctx = context();
		const { commands } = buildBuiltInCommands(ctx);

		commands.find((command) => command.id === "tasks")?.run();
		commands.find((command) => command.id === "diagnostic")?.run();

		expect(ctx.actions.openWorkspace).toHaveBeenNthCalledWith(1, "tasks");
		expect(ctx.actions.toggleDiagnostic).toHaveBeenCalledOnce();
		expect(ctx.actions.openWorkspace).toHaveBeenNthCalledWith(2, "diagnostics");
	});

	it("preserves dynamic labels without giving modules direct state access", () => {
		const ctx = context();
		ctx.microphoneLabel = "Stop listening";
		ctx.diagnosticRunning = true;
		ctx.motionEnabled = false;

		const { commands } = buildBuiltInCommands(ctx);

		expect(commands.find((command) => command.id === "voice")?.label).toBe(
			"Stop listening",
		);
		expect(commands.find((command) => command.id === "diagnostic")?.label).toBe(
			"Cancel diagnostic",
		);
		expect(commands.find((command) => command.id === "motion")?.label).toBe(
			"Resume animations",
		);
	});
});
