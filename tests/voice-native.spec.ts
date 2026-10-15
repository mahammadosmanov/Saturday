import { expect, test } from "@playwright/test";
import { installNativeProtocolTestDriver } from "./fixtures/voice/native-protocol-driver";

test("main composer waits for an actual protocol snapshot, then submits one validated request", async ({ page }) => {
	await installNativeProtocolTestDriver(page, { holdSnapshot: true });
	await page.goto("/");
	const composer = page.locator("#main-request");
	await composer.fill("hello local assistant");
	await composer.press("Enter");
	await expect(page.getByRole("dialog", { name: "Voice conversation" })).toBeVisible();
	await expect.poll(async () => page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.calls.filter((call) => call.command === "voice_submit_text").length)).toBe(0);
	await page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.releaseSnapshot());
	await expect.poll(async () => page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.calls.filter((call) => call.command === "voice_submit_text").length)).toBe(1);
	await expect(page.locator(".conversation-messages .message.assistant")).toBeVisible();
	const submit = await page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.calls.find((call) => call.command === "voice_submit_text"));
	expect(submit?.request.text).toBe("hello local assistant");
	await expect(page.locator("#main-request")).toHaveValue("");
});

test("closing while the initial snapshot is pending cancels the queued composer request", async ({ page }) => {
	await installNativeProtocolTestDriver(page, { holdSnapshot: true });
	await page.goto("/");
	await page.locator("#main-request").fill("do not submit after close");
	await page.locator("#main-request").press("Enter");
	await page.getByRole("button", { name: "Close dialog" }).click();
	await page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.releaseSnapshot());
	await expect(page.getByRole("dialog")).toHaveCount(0);
	await expect.poll(async () => page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.calls.filter((call) => call.command === "voice_submit_text").length)).toBe(0);
	await expect(page.locator("#main-request")).toHaveValue("do not submit after close");
});

test("unsupported browser mode exposes no assistant or browser-media fallback", async ({ page }) => {
	await page.goto("/");
	await page.getByRole("button", { name: "Open conversation", exact: true }).click();
	await expect(page.getByRole("dialog").getByRole("status")).toBeVisible();
	await expect(page.getByRole("button", { name: "Send typed request" })).toBeDisabled();
	await expect(page.locator("audio, video, input[type='file']")).toHaveCount(0);
});

test("invalid main-composer input cannot open or submit a native turn", async ({ page }) => {
	await installNativeProtocolTestDriver(page);
	await page.goto("/");
	const composer = page.locator("#main-request");
	await composer.fill("x".repeat(501));
	await composer.press("Enter");
	await expect(page.getByRole("dialog")).toHaveCount(0);
	await expect.poll(async () => page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.calls.length)).toBe(0);
});

for (const [width, height] of [[600, 600], [960, 720]] as const) {
	test(`native conversation controls fit ${width}x${height} with reduced motion and 44px targets`, async ({ page }) => {
		await page.setViewportSize({ width, height });
		await page.emulateMedia({ reducedMotion: "reduce" });
		await installNativeProtocolTestDriver(page);
		await page.goto("/");
		await page.getByRole("button", { name: "Open conversation", exact: true }).click();
		const dialog = page.getByRole("dialog", { name: "Voice conversation" });
		await expect(dialog).toBeVisible();
		await expect(dialog.locator("#voice-typed-request")).toBeEditable();
		await expect(dialog.locator("#voice-typed-request")).toBeFocused();
		const bounds = await dialog.boundingBox();
		if (!bounds) throw new Error("Conversation dialog has no rendered bounds");
		expect(bounds.width).toBeLessThanOrEqual(width);
		expect(bounds.height).toBeLessThanOrEqual(height);
		const action = dialog.locator(".voice-control-row button").first();
		await expect(action).toBeVisible();
		const target = await action.boundingBox();
		if (!target) throw new Error("Voice action has no rendered bounds");
		expect(target.height).toBeGreaterThanOrEqual(44);
	});
}

test("closing the voice surface preserves its native-owned session history", async ({ page }) => {
	await installNativeProtocolTestDriver(page);
	await page.goto("/");
	await page.getByRole("button", { name: "Open conversation", exact: true }).click();
	const input = page.locator("#voice-typed-request");
	await input.fill("keep this session message");
	await page.getByRole("button", { name: "Send typed request" }).click();
	await expect(page.locator(".conversation-messages .message.user").filter({ hasText: "keep this session message" })).toBeVisible();
	await expect(page.locator(".conversation-messages .message.assistant")).toBeVisible();
	await page.getByRole("button", { name: "Close dialog" }).click();
	await expect(page.getByRole("dialog")).toHaveCount(0);
	await page.getByRole("button", { name: "Open conversation", exact: true }).click();
	await expect(page.getByText("keep this session message", { exact: true })).toBeVisible();
	await expect(page.locator(".conversation-messages .message.user").filter({ hasText: "keep this session message" })).toBeVisible();
	await expect(page.locator(".conversation-messages .message.assistant")).toBeVisible();
});
test("closing the conversation surface waits for native turn cancellation and cleanup", async ({ page }) => {
	await installNativeProtocolTestDriver(page);
	await page.goto("/");
	await page.getByRole("button", { name: "Open conversation", exact: true }).click();
	await page.getByRole("button", { name: "Start voice conversation", exact: true }).click();
	await expect.poll(async () => page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.calls.filter((call) => call.command === "voice_start").length)).toBe(1);
	await page.getByRole("button", { name: "Close dialog" }).click();
	await expect(page.getByRole("dialog")).toHaveCount(0);
	await expect.poll(async () => page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.calls.filter((call) => call.command === "voice_cancel").length)).toBe(1);
	const cancel = await page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.calls.find((call) => call.command === "voice_cancel"));
	expect(cancel?.request.reason).toBe("surface_closed");
});


test("switching to the unrelated command palette waits for active native-turn cancellation", async ({ page }) => {
	await installNativeProtocolTestDriver(page);
	await page.goto("/");
	await page.getByRole("button", { name: "Open conversation", exact: true }).click();
	await page.getByRole("button", { name: "Start voice conversation", exact: true }).click();
	await expect.poll(async () => page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.calls.filter((call) => call.command === "voice_start").length)).toBe(1);
	await expect(page.getByRole("button", { name: "Cancel voice turn" })).toBeEnabled();
	await page.keyboard.press("Meta+k");
	await expect(page.getByRole("dialog", { name: "Command palette" })).toBeVisible();
	const cancel = await page.evaluate(() => window.__SATURDAY_VOICE_PROTOCOL_TEST_DRIVER__.calls.find((call) => call.command === "voice_cancel"));
	expect(cancel?.request.reason).toBe("module_disabled");
});
