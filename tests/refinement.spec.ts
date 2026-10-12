import { test, expect } from "@playwright/test";
test("header omits removed surfaces and the core uses dotted branding", async ({
	page,
}) => {
	await page.goto("/");
	const header = page.locator(".minimal-header");
	await expect(header.getByText("UI PREVIEW", { exact: true })).toHaveCount(0);
	await expect(header.locator(".emblem, .clock, time")).toHaveCount(0);
	await expect(header.getByRole("button")).toHaveCount(0);
	await expect(
		page.getByRole("heading", { name: "S.A.T.U.R.D.A.Y", exact: true }),
	).toBeVisible();
});
const orbit = '[data-orbit="inner"], .orbit-inner';
test("compact primary surface keeps one composer and no removed navigation", async ({
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
	for (const label of ["Social Studio", "Workspace", "Settings"]) {
		await expect(page.getByRole("button", { name: label, exact: true })).toHaveCount(
			0,
		);
	}
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
test("opening Conversation never recreates the core", async ({ page }) => {
	await page.goto("/");
	const canvas = await page.locator("canvas.core-particles").elementHandle();
	const player = await page
		.locator(orbit)
		.evaluateHandle((el) => el.getAnimations()[0]);
	await page
		.getByRole("button", { name: "Open conversation", exact: true })
		.click();
	await expect(
		page.getByRole("dialog", { name: "Conversation" }),
	).toBeVisible();
	await page.keyboard.press("Escape");
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
		page.getByText(
			"The local interface is online. This reply is scripted. No AI provider is connected.",
			{ exact: true },
		),
	).toBeVisible();
	await page.keyboard.press("Escape");
	expect(
		await page
			.locator("canvas.core-particles")
			.evaluate((el, old) => el === old, canvas),
	).toBe(true);
	await expect(input).toBeFocused();
});
test("saved theme retains canvas geometry without a Settings surface", async ({
	page,
}) => {
	await page.addInitScript(() =>
		localStorage.setItem(
			"saturday.workspace.v1",
			JSON.stringify({ version: 1, accent: "amber" }),
		),
	);
	await page.goto("/");
	await expect(page.locator(".app")).toHaveAttribute("data-accent", "amber");
	const layer = page.locator('[data-orbit="inner"]');
	const player = await layer.evaluateHandle((el) => el.getAnimations()[0]);
	await player.evaluate((a) => a.ready);
	const canvas = page.locator("canvas.core-particles");
	const original = await canvas.elementHandle();
	await page
		.getByRole("button", { name: "Open command palette", exact: true })
		.click();
	await page.getByRole("option", { name: /^Pause animations/ }).click();
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	await expect
		.poll(() => player.evaluate((animation) => animation.playState))
		.toBe("paused");
	const time = await player.evaluate((animation) => animation.currentTime);
	const pixels = await canvas.evaluateHandle((el) => {
		const c = el as HTMLCanvasElement;
		return c.getContext("2d")?.getImageData(0, 0, c.width, c.height).data;
	});
	await page.waitForTimeout(100);
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
