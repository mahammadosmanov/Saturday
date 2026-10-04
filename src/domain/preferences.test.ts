import { describe, expect, it } from "vitest";
import { parseWorkspace } from "./preferences";

describe("workspace preferences", () => {
	it("preserves an exact microphone preference and safely defaults legacy/invalid values", () => {
		expect(
			parseWorkspace(
				JSON.stringify({ version: 1, microphoneDeviceId: "MiC-CaseSensitive" }),
			).microphoneDeviceId,
		).toBe("MiC-CaseSensitive");
		for (const microphoneDeviceId of [undefined, null, 42, "x".repeat(513)])
			expect(
				parseWorkspace(JSON.stringify({ version: 1, microphoneDeviceId }))
					.microphoneDeviceId,
			).toBe("");
	});
	it("preserves valid preferences and rejects malformed saved fields", () => {
		const saved = parseWorkspace(
			JSON.stringify({
				version: 1,
				accent: "amber",
				motion: false,
				density: "compact",
				widgets: { telemetry: false, network: "false" },
				objectives: [
					{ id: "a", text: "  Build something  ", done: true },
					{ id: "a", text: "Duplicate", done: false },
					{ id: "b", text: "x".repeat(400), done: "yes" },
					null,
				],
			}),
		);
		expect(saved.accent).toBe("amber");
		expect(saved.motion).toBe(false);
		expect(saved.density).toBe("compact");
		expect(saved.widgets.telemetry).toBe(false);
		expect(saved.widgets.network).toBe(true);
		expect(saved.objectives).toEqual([
			{ id: "a", text: "Build something", done: true },
			{ id: "b", text: "x".repeat(100), done: false },
		]);
		expect(parseWorkspace('{"version":1,"objectives":[]}').objectives).toEqual(
			[],
		);
		expect(
			parseWorkspace(
				JSON.stringify({
					version: 1,
					objectives: Array.from({ length: 40 }, (_, n) => ({
						id: String(n),
						text: "A",
						done: false,
					})),
				}),
			).objectives,
		).toHaveLength(20);
	});
	it("restores safe defaults when no valid saved workspace exists", () => {
		for (const raw of [null, "{", "null", "[]", '{"version":99}']) {
			const workspace = parseWorkspace(raw);
			expect(workspace.accent).toBe("cyan");
			expect(workspace.motion).toBe(true);
			expect(workspace.density).toBe("comfortable");
			expect(workspace.widgets).toEqual({
				telemetry: true,
				network: true,
				activity: true,
				objectives: true,
			});
		}
	});
});
