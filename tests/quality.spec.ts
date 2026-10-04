import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
for (const [width, height] of [
	[320, 1000],
	[390, 844],
	[768, 1024],
	[1024, 1000],
	[1280, 800],
	[1440, 900],
	[1440, 1000],
	[1920, 1080],
]) {
	test(`compact layout and drawers fit ${width}x${height}`, async ({
		page,
	}) => {
		await page.setViewportSize({ width, height });
		await page.goto("/");
		await expect(
			page.getByRole("region", { name: "Assistant core" }),
		).toBeVisible();
		expect(
			await page.evaluate(() => document.documentElement.scrollWidth),
		).toBeLessThanOrEqual(width);
		await page
			.getByRole("button", { name: "Open conversation", exact: true })
			.click();
		await expect(
			page.getByRole("textbox", { name: "Message J.A.R.V.I.S" }),
		).toBeVisible();
		expect(
			await page.evaluate(() => document.documentElement.scrollWidth),
		).toBeLessThanOrEqual(width);
		await page.keyboard.press("Escape");
		await page.getByRole("button", { name: "Workspace", exact: true }).click();
		await page
			.getByRole("button", { name: "Diagnostics", exact: true })
			.click();
		await expect(
			page.getByRole("heading", { name: "Diagnostics", exact: true }),
		).toBeVisible();
		expect(
			await page.evaluate(() => document.documentElement.scrollWidth),
		).toBeLessThanOrEqual(width);
	});
}
test("all compact surfaces and dialogs pass automated WCAG A/AA checks", async ({
	page,
}) => {
	await page.emulateMedia({ reducedMotion: "reduce" });
	await page.goto("/");
	const check = async (label: string) => {
		const result = await new AxeBuilder({ page })
			.withTags(["wcag2a", "wcag2aa", "wcag21aa"])
			.analyze();
		expect(result.violations, label).toEqual([]);
	};
	await check("compact home");
	await page.getByRole("button", { name: "Workspace", exact: true }).click();
	for (const label of ["Tasks", "Diagnostics", "Activity"]) {
		await page.getByRole("button", { name: label, exact: true }).click();
		await check(label);
	}
	await page.keyboard.press("Escape");
	await page.getByRole("button", { name: "Settings", exact: true }).click();
	await check("settings");
	await page.getByRole("button", { name: "Amber", exact: true }).click();
	await check("amber settings");
	await page.keyboard.press("Escape");
	await check("amber home");
	await page.keyboard.press("Meta+k");
	await check("command palette");
	await page.keyboard.press("Escape");
	await page
		.getByRole("button", { name: "Open conversation", exact: true })
		.click();
	await check("conversation");
});
test("settings trap and restore focus, reset requires confirmation, corruption is disclosed", async ({
	page,
}) => {
	await page.addInitScript(() =>
		localStorage.setItem("jarvis.workspace.v1", "{malformed"),
	);
	await page.goto("/");
	await expect(
		page.getByText(
			"Saved workspace was unreadable. Safe defaults were restored.",
			{ exact: true },
		),
	).toBeVisible();
	await page.getByRole("button", { name: "Settings", exact: true }).click();
	for (let i = 0; i < 18; i++) {
		await page.keyboard.press("Tab");
		expect(
			await page
				.getByRole("dialog")
				.evaluate((el) => el.contains(document.activeElement)),
		).toBe(true);
	}
	await page.getByRole("button", { name: "Amber", exact: true }).click();
	await page
		.getByRole("button", { name: "Reset workspace", exact: true })
		.click();
	await page
		.getByRole("button", { name: "Keep workspace", exact: true })
		.click();
	await expect(page.locator(".app")).toHaveAttribute("data-accent", "amber");
	await page
		.getByRole("button", { name: "Reset workspace", exact: true })
		.click();
	await page
		.getByRole("button", { name: "Confirm reset", exact: true })
		.click();
	await expect(page.locator(".app")).toHaveAttribute("data-accent", "cyan");
	await page.keyboard.press("Escape");
	await expect(
		page.getByRole("button", { name: "Settings", exact: true }),
	).toBeFocused();
});
test("unavailable storage is disclosed and session-only tasks remain usable", async ({
	page,
}) => {
	await page.addInitScript(() => {
		Storage.prototype.getItem = () => {
			throw new DOMException("Blocked", "SecurityError");
		};
		Storage.prototype.setItem = () => {
			throw new DOMException("Blocked", "SecurityError");
		};
	});
	await page.goto("/");
	await expect(
		page.getByText("Changes are session-only. Local storage is unavailable.", {
			exact: true,
		}),
	).toBeVisible();
	await page.getByRole("button", { name: "Workspace", exact: true }).click();
	await page
		.getByRole("checkbox", { name: "Run a simulated diagnostic" })
		.check();
	await expect(
		page.getByRole("checkbox", { name: "Run a simulated diagnostic" }),
	).toBeChecked();
});
test("local runtime makes no external requests and diagnostics work offline", async ({
	page,
	context,
}) => {
	const external: string[] = [],
		errors: string[] = [];
	page.on("request", (request) => {
		if (
			/^https?:/.test(request.url()) &&
			!request.url().startsWith("http://127.0.0.1:1420/")
		)
			external.push(request.url());
	});
	page.on("pageerror", (error) => errors.push(error.message));
	await page.goto("/");
	await page.getByRole("button", { name: "Workspace", exact: true }).click();
	await page.getByRole("button", { name: "Diagnostics", exact: true }).click();
	await context.setOffline(true);
	await page
		.getByRole("button", { name: "Run diagnostic", exact: true })
		.click();
	await expect(
		page.getByText("Simulation complete", { exact: true }),
	).toBeVisible({ timeout: 10000 });
	await page.getByRole("button", { name: "Activity", exact: true }).click();
	await expect(
		page.getByText("Diagnostic simulation complete", { exact: true }),
	).toBeVisible();
	expect(errors).toEqual([]);
	expect(external).toEqual([]);
});
