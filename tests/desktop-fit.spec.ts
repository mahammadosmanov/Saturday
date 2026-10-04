import { test, expect } from "@playwright/test";
for (const [width, height] of [
	[960, 720],
	[1440, 1000],
]) {
	test(`compact home fits ${width} by ${height} without scrolling`, async ({
		page,
	}) => {
		await page.setViewportSize({ width, height });
		await page.goto("/");
		await page.evaluate(() => document.fonts.ready);
		expect(
			await page
				.locator(".composer-foot")
				.evaluate((el) => el.getBoundingClientRect().bottom),
		).toBeLessThanOrEqual(height);
		expect(
			await page.evaluate(() => document.documentElement.scrollHeight),
		).toBeLessThanOrEqual(height);
	});
}
