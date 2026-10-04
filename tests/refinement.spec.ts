import { test, expect } from "@playwright/test";
test("header keeps Social Studio, workspace and settings without preview branding clock or pause", async ({
	page,
}) => {
	await page.goto("/");
	const header = page.locator(".minimal-header");
	await expect(header.getByText("UI PREVIEW", { exact: true })).toHaveCount(0);
	await expect(header.locator(".emblem, .clock, time")).toHaveCount(0);
	await expect(header.getByRole("button")).toHaveCount(3);
	await expect(
		header.getByRole("button", { name: "Social Studio", exact: true }),
	).toBeVisible();
	await expect(
		header.getByRole("button", { name: "Workspace", exact: true }),
	).toBeVisible();
	await expect(
		header.getByRole("button", { name: "Settings", exact: true }),
	).toBeVisible();
});
const orbit = '[data-orbit="inner"], .orbit-inner';
test("compact primary surface has one composer and no decorative dashboards", async ({
	page,
}) => {
	await page.goto("/");
	await expect(
		page.getByRole("region", { name: "System telemetry", exact: true }),
	).toHaveCount(0);
	await expect(
		page.getByRole("region", { name: "Signal topology", exact: true }),
	).toHaveCount(0);
	await expect(
		page.getByRole("textbox", { name: "Command message" }),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Workspace", exact: true }),
	).toBeVisible();
	expect(await page.locator("main button").count()).toBeLessThanOrEqual(4);
});
test("pause and resume preserve the exact ring player and its phase", async ({
	page,
}) => {
	await page.goto("/");
	const layer = page.locator(orbit);
	const player = await layer.evaluateHandle((el) => el.getAnimations()[0]);
	await player.evaluate((a) => {
		a.currentTime = 6000;
	});
	await page
		.getByRole("button", { name: "Open command palette", exact: true })
		.click();
	await page.getByRole("option", { name: /^Pause animations/ }).click();
	await expect(
		page.getByRole("dialog", { name: "Command palette" }),
	).toHaveCount(0);
	expect(
		await layer.evaluate((el, old) => el.getAnimations()[0] === old, player),
	).toBe(true);
	await player.evaluate((a) => a.ready);
	const paused = await player.evaluate((a) => Number(a.currentTime));
	await page.waitForTimeout(200);
	expect(await player.evaluate((a) => Number(a.currentTime))).toBeCloseTo(
		paused,
		1,
	);
	await page
		.getByRole("button", { name: "Open command palette", exact: true })
		.click();
	await page.getByRole("option", { name: /^Resume animations/ }).click();
	await expect(
		page.getByRole("dialog", { name: "Command palette" }),
	).toHaveCount(0);
	expect(
		await layer.evaluate((el, old) => el.getAnimations()[0] === old, player),
	).toBe(true);
	await expect
		.poll(() => player.evaluate((a) => Number(a.currentTime)))
		.toBeGreaterThan(paused);
});
test("assistant state changes adjust speed without seeking the ring timeline", async ({
	page,
}) => {
	await page.goto("/");
	const layer = page.locator(orbit);
	const player = await layer.evaluateHandle((el) => el.getAnimations()[0]);
	await player.evaluate((a) => {
		a.pause();
		a.currentTime = 6000;
	});
	const before = await player.evaluate((a) => ({
		duration: a.effect?.getTiming().duration,
		phase: a.effect?.getComputedTiming().progress,
	}));
	if (
		await page.getByRole("button", { name: "Workspace", exact: true }).count()
	) {
		await page.getByRole("button", { name: "Workspace", exact: true }).click();
		await page
			.getByRole("button", { name: "Diagnostics", exact: true })
			.click();
	}
	await page
		.getByRole("button", { name: "Run diagnostic", exact: true })
		.click();
	await expect(page.locator(".core-panel")).toHaveAttribute(
		"data-state",
		"scanning",
	);
	const after = await player.evaluate((a) => ({
		duration: a.effect?.getTiming().duration,
		phase: a.effect?.getComputedTiming().progress,
	}));
	expect(after.duration).toBe(before.duration);
	expect(after.phase).toBeCloseTo(before.phase ?? -1, 4);
});
test("opening and navigating secondary tools never recreates the core", async ({
	page,
}) => {
	await page.goto("/");
	const canvas = await page.locator("canvas.core-particles").elementHandle();
	const player = await page
		.locator(orbit)
		.evaluateHandle((el) => el.getAnimations()[0]);
	if (
		await page.getByRole("button", { name: "Workspace", exact: true }).count()
	) {
		await page.getByRole("button", { name: "Workspace", exact: true }).click();
		await page
			.getByRole("button", { name: "Diagnostics", exact: true })
			.click();
		await page.getByRole("button", { name: "Activity", exact: true }).click();
		await page.keyboard.press("Escape");
	} else {
		await page.getByRole("button", { name: "Systems", exact: true }).click();
		await page.getByRole("button", { name: "Overview", exact: true }).click();
	}
	expect(
		await page
			.locator("canvas.core-particles")
			.evaluate((el, old) => el === old, canvas),
	).toBe(true);
	expect(
		await page
			.locator(orbit)
			.evaluate((el, old) => el.getAnimations()[0] === old, player),
	).toBe(true);
});
test("visibility and reduced motion freeze pixels and resume without replacing timelines", async ({
	page,
}) => {
	await page.goto("/");
	const layer = page.locator(orbit);
	const player = await layer.evaluateHandle((el) => el.getAnimations()[0]);
	await page.evaluate(() => {
		Object.defineProperty(document, "visibilityState", {
			configurable: true,
			value: "hidden",
		});
		document.dispatchEvent(new Event("visibilitychange"));
	});
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	expect(
		await layer.evaluate((el, old) => el.getAnimations()[0] === old, player),
	).toBe(true);
	await player.evaluate((a) => a.ready);
	const canvas = page.locator("canvas.core-particles");
	const image = await canvas.evaluate((el) =>
		(el as HTMLCanvasElement).toDataURL(),
	);
	await page.waitForTimeout(200);
	expect(
		await canvas.evaluate((el) => (el as HTMLCanvasElement).toDataURL()),
	).toBe(image);
	await page.evaluate(() => {
		Reflect.deleteProperty(document, "visibilityState");
		document.dispatchEvent(new Event("visibilitychange"));
	});
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "on");
	await expect.poll(() => player.evaluate((a) => a.playState)).toBe("running");
	await page.emulateMedia({ reducedMotion: "reduce" });
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	await expect.poll(() => player.evaluate((a) => a.playState)).toBe("paused");
});
test("main composer submits directly and keeps the core alive through a response", async ({
	page,
}) => {
	await page.goto("/");
	const canvas = await page.locator("canvas.core-particles").elementHandle();
	const input = page.getByRole("textbox", { name: "Command message" });
	await input.fill("status");
	await input.press("Enter");
	await expect(
		page.getByRole("dialog", { name: "Conversation" }),
	).toBeVisible();
	await expect(
		page.getByText(/saved settings, and checklist are real/),
	).toBeVisible();
	await page.keyboard.press("Escape");
	expect(
		await page
			.locator("canvas.core-particles")
			.evaluate((el, old) => el === old, canvas),
	).toBe(true);
	await expect(input).toBeFocused();
});
test("theme changes preserve the canvas geometry and ring positions", async ({
	page,
}) => {
	await page.goto("/");
	await page
		.getByRole("button", { name: "Open command palette", exact: true })
		.click();
	await page.getByRole("option", { name: /^Pause animations/ }).click();
	await expect(
		page.getByRole("dialog", { name: "Command palette" }),
	).toHaveCount(0);
	const layer = page.locator('[data-orbit="inner"]');
	const player = await layer.evaluateHandle((el) => el.getAnimations()[0]);
	await player.evaluate((a) => a.ready);
	const time = await player.evaluate((a) => a.currentTime);
	const canvas = page.locator("canvas.core-particles");
	const original = await canvas.elementHandle();
	const pixels = await canvas.evaluateHandle((el) => {
		const c = el as HTMLCanvasElement;
		return c.getContext("2d")?.getImageData(0, 0, c.width, c.height).data;
	});
	await page.getByRole("button", { name: "Settings", exact: true }).click();
	await page.getByRole("button", { name: "Amber", exact: true }).click();
	await page.keyboard.press("Escape");
	expect(
		await layer.evaluate((el, old) => el.getAnimations()[0] === old, player),
	).toBe(true);
	expect(await player.evaluate((a) => a.currentTime)).toBe(time);
	expect(await canvas.evaluate((el, old) => el === old, original)).toBe(true);
	expect(
		await canvas.evaluate((el, before) => {
			const c = el as HTMLCanvasElement,
				after = c.getContext("2d")?.getImageData(0, 0, c.width, c.height).data;
			if (!before || !after) return false;
			return after.every((alpha, i) => i % 4 !== 3 || alpha === before[i]);
		}, pixels),
	).toBe(true);
});
