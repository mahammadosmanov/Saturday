import { describe, expect, it, vi } from "vitest";
import {
	ConversationRuntime,
	createScriptedConversationProvider,
	type ConversationProvider,
} from "./conversation";

describe("ConversationRuntime", () => {
	it("normalizes bounded input before calling a provider", async () => {
		const respond = vi.fn(async ({ text }: { text: string }) => ({
			text: `reply:${text}`,
		}));
		const provider: ConversationProvider = {
			metadata: {
				id: "test.provider",
				name: "Test provider",
				mode: "scripted",
				locality: "local",
			},
			respond,
		};
		const runtime = new ConversationRuntime(provider);
		const input = `  ${"x".repeat(600)}  `;

		const reply = await runtime.respond(input, new AbortController().signal);

		expect(respond).toHaveBeenCalledWith(
			{ text: "x".repeat(500) },
			expect.objectContaining({ signal: expect.any(AbortSignal) }),
		);
		expect(reply?.text).toBe(`reply:${"x".repeat(500)}`);
	});

	it("does not call a provider for empty input", async () => {
		const respond = vi.fn();
		const provider: ConversationProvider = {
			metadata: {
				id: "test.provider",
				name: "Test provider",
				mode: "scripted",
				locality: "local",
			},
			respond,
		};
		const runtime = new ConversationRuntime(provider);

		await expect(
			runtime.respond("   ", new AbortController().signal),
		).resolves.toBeNull();
		expect(respond).not.toHaveBeenCalled();
	});

	it("propagates cancellation to a pending provider", async () => {
		vi.useFakeTimers();
		const runtime = new ConversationRuntime(
			createScriptedConversationProvider({ delayMs: 850 }),
		);
		const controller = new AbortController();
		const response = runtime.respond("status", controller.signal);
		const cancelled = expect(response).rejects.toMatchObject({
			name: "AbortError",
		});

		controller.abort();
		await vi.runAllTimersAsync();

		await cancelled;
		vi.useRealTimers();
	});
});

describe("scripted conversation provider", () => {
	it("keeps focus but does not expose the removed workspace workflow", async () => {
		const runtime = new ConversationRuntime(
			createScriptedConversationProvider({ delayMs: 0 }),
		);
		const signal = new AbortController().signal;

		const removedWorkflowReply = await runtime.respond(
			"run a diagnostic",
			signal,
		);
		expect(removedWorkflowReply?.action).toBeUndefined();
		expect(removedWorkflowReply?.text).not.toMatch(/workspace|task|diagnostic/i);
		await expect(runtime.respond("focus", signal)).resolves.toEqual(
			expect.objectContaining({ action: "focus" }),
		);

		const graphReply = await runtime.respond("open the graph", signal);
		expect(graphReply?.action).toBeUndefined();
		expect(graphReply?.text).toContain("not connected to an AI model");
	});
});
