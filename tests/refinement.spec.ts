import { expect, test } from "@playwright/test";

const orbit = '[data-orbit="inner"]';

test("core keeps dotted branding without obsolete chrome", async ({ page }) => {
	await page.goto("/");
	await expect(page.getByRole("heading", { name: "S.A.T.U.R.D.A.Y", exact: true })).toBeVisible();
	await expect(page.locator(".minimal-header .emblem, .minimal-header .clock, .minimal-header time")).toHaveCount(0);
});

test("compact primary surface keeps one composer and no removed navigation", async ({ page }) => {
	await page.goto("/");
	await expect(page.getByRole("region", { name: "System telemetry", exact: true })).toHaveCount(0);
	await expect(page.getByRole("region", { name: "Signal topology", exact: true })).toHaveCount(0);
	await expect(page.getByRole("textbox", { name: "Request to the local assistant" })).toBeVisible();
	for (const label of ["Social Studio", "Workspace", "Settings"])
		await expect(page.getByRole("button", { name: label, exact: true })).toHaveCount(0);
	expect(await page.locator("main button").count()).toBeLessThanOrEqual(4);
});

test("pausing and resuming keeps the orbit animation timeline", async ({ page }) => {
	await page.goto("/");
	const layer = page.locator(orbit);
	const player = await layer.evaluateHandle((element) => element.getAnimations()[0]);
	await player.evaluate((animation) => { animation.currentTime = 6000; });
	await page.locator("#command-trigger").click();
	await page.getByRole("option", { name: /^Pause animations/ }).click();
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	await expect.poll(() => player.evaluate((animation) => animation.playState)).toBe("paused");
	expect(await layer.evaluate((element, old) => element.getAnimations()[0] === old, player)).toBe(true);
	const paused = await player.evaluate((animation) => Number(animation.currentTime));
	await page.waitForTimeout(200);
	expect(await player.evaluate((animation) => Number(animation.currentTime))).toBeCloseTo(paused, 1);
	await page.locator("#command-trigger").click();
	await page.getByRole("option", { name: /^Resume animations/ }).click();
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "on");
	await expect.poll(() => player.evaluate((animation) => animation.playState)).toBe("running");
});

test("opening the voice surface does not replace the core", async ({ page }) => {
	await page.goto("/");
	const core = await page.locator(".core-panel").elementHandle();
	if (!core) throw new Error("Core panel was not rendered");
	const layer = page.locator(orbit);
	const player = await layer.evaluateHandle((element) => element.getAnimations()[0]);
	await page.getByRole("button", { name: "Open conversation", exact: true }).click();
	await expect(page.getByRole("dialog", { name: "Voice conversation" })).toBeVisible();
	await page.keyboard.press("Escape");
	await expect(page.getByRole("dialog")).toHaveCount(0);
	expect(await page.locator(".core-panel").evaluate((element, old) => element === old, core)).toBe(true);
	expect(await layer.evaluate((element, old) => element.getAnimations()[0] === old, player)).toBe(true);
});

test("visibility and reduced motion pause and resume the same orbit timeline", async ({ page }) => {
	await page.goto("/");
	const layer = page.locator(orbit);
	const player = await layer.evaluateHandle((element) => element.getAnimations()[0]);
	await player.evaluate((animation) => { animation.currentTime = 6000; });
	await page.evaluate(() => {
		Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
		document.dispatchEvent(new Event("visibilitychange"));
	});
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	await expect.poll(() => player.evaluate((animation) => animation.playState)).toBe("paused");
	expect(await layer.evaluate((element, old) => element.getAnimations()[0] === old, player)).toBe(true);
	const paused = await player.evaluate((animation) => Number(animation.currentTime));
	await page.waitForTimeout(200);
	expect(await player.evaluate((animation) => Number(animation.currentTime))).toBeCloseTo(paused, 1);
	await page.evaluate(() => {
		Reflect.deleteProperty(document, "visibilityState");
		document.dispatchEvent(new Event("visibilitychange"));
	});
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "on");
	await expect.poll(() => player.evaluate((animation) => animation.playState)).toBe("running");
	await page.emulateMedia({ reducedMotion: "reduce" });
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	await expect.poll(() => player.evaluate((animation) => animation.playState)).toBe("paused");
});

test("saved accent survives motion changes without a settings surface", async ({ page }) => {
	await page.addInitScript(() => localStorage.setItem("saturday.workspace.v1", JSON.stringify({ version: 1, accent: "amber" })));
	await page.goto("/");
	await expect(page.locator(".app")).toHaveAttribute("data-accent", "amber");
	await page.locator("#command-trigger").click();
	await page.getByRole("option", { name: /^Pause animations/ }).click();
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	await expect(page.locator(".app")).toHaveAttribute("data-accent", "amber");
	await expect(page.getByRole("button", { name: "Open conversation", exact: true })).toBeVisible();
});
