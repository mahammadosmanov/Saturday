import { test, expect } from "@playwright/test";
test("paused canvas stays painted and stationary under explicit and reduced motion", async ({
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
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	const canvas = page.locator("canvas.core-particles");
	const image = await canvas.evaluate((el) =>
		(el as HTMLCanvasElement).toDataURL(),
	);
	await page.waitForTimeout(200);
	expect(
		await canvas.evaluate((el) => (el as HTMLCanvasElement).toDataURL()),
	).toBe(image);
	await page
		.getByRole("button", { name: "Open command palette", exact: true })
		.click();
	await page.getByRole("option", { name: /^Resume animations/ }).click();
	await expect(
		page.getByRole("dialog", { name: "Command palette" }),
	).toHaveCount(0);
	await page.emulateMedia({ reducedMotion: "reduce" });
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
	await expect
		.poll(() =>
			page
				.locator('[data-orbit="inner"]')
				.evaluate((el) => el.getAnimations()[0].playState),
		)
		.toBe("paused");
});
