import { test, expect } from "@playwright/test";

test("explicit and reduced-motion settings pause the orbit animation", async ({ page }) => {
	await page.goto("/");
	const orbit = page.locator('[data-orbit="inner"]');
	const animation = await orbit.evaluateHandle((element) => element.getAnimations()[0]);
	await page.locator("#command-trigger").click();
	await page.getByRole("option", { name: /^Pause animations/ }).click();
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	await expect.poll(() => animation.evaluate((current) => current.playState)).toBe("paused");
	const pausedTime = await animation.evaluate((current) => Number(current.currentTime));
	await page.waitForTimeout(200);
	expect(await animation.evaluate((current) => Number(current.currentTime))).toBeCloseTo(pausedTime, 1);
	await page.locator("#command-trigger").click();
	await page.getByRole("option", { name: /^Resume animations/ }).click();
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "on");
	await expect.poll(() => animation.evaluate((current) => current.playState)).toBe("running");
	await page.emulateMedia({ reducedMotion: "reduce" });
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	await expect.poll(() => animation.evaluate((current) => current.playState)).toBe("paused");
});
