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
	test(`retained surfaces fit ${width}x${height}`, async ({ page }) => {
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
			page.getByRole("textbox", { name: "Message S.A.T.U.R.D.A.Y" }),
		).toBeVisible();
		expect(
			await page.evaluate(() => document.documentElement.scrollWidth),
		).toBeLessThanOrEqual(width);
		await page.keyboard.press("Escape");
		await page.keyboard.press("Meta+k");
		await expect(
			page.getByRole("dialog", { name: "Command palette" }),
		).toBeVisible();
		expect(
			await page.evaluate(() => document.documentElement.scrollWidth),
		).toBeLessThanOrEqual(width);
	});
}

test("retained compact surfaces pass automated WCAG A/AA checks", async ({
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
	for (const label of ["Social Studio", "Workspace", "Settings"]) {
		await expect(page.getByRole("button", { name: label, exact: true })).toHaveCount(
			0,
		);
	}
	await page.keyboard.press("Meta+k");
	await check("command palette");
	await page.keyboard.press("Escape");
	await page
		.getByRole("button", { name: "Open conversation", exact: true })
		.click();
	await check("conversation");
});

test("unreadable saved preferences are disclosed without restoring removed controls", async ({
	page,
}) => {
	await page.addInitScript(() =>
		localStorage.setItem("saturday.workspace.v1", "{malformed"),
	);
	await page.goto("/");
	await expect(
		page.getByText(
			"Saved preferences were unreadable. Safe defaults were restored.",
			{ exact: true },
		),
	).toBeVisible();
	await page.locator("#command-trigger").focus();
	await page.keyboard.press("Meta+k");
	const search = page.getByRole("combobox", { name: "Search commands" });
	await search.fill("pause animations");
	await search.press("Enter");
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
});

test("session-only motion control still works when storage is unavailable", async ({
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
	await page.locator("#command-trigger").focus();
	await page.keyboard.press("Meta+k");
	const search = page.getByRole("combobox", { name: "Search commands" });
	await search.fill("pause animations");
	await search.press("Enter");
	await expect(page.locator(".app")).toHaveAttribute("data-motion", "off");
});

test("scripted conversation works offline without external requests", async ({
	page,
	context,
}) => {
	const external: string[] = [];
	const errors: string[] = [];
	page.on("request", (request) => {
		if (
			/^https?:/.test(request.url()) &&
			!request.url().startsWith("http://127.0.0.1:1420/")
		)
			external.push(request.url());
	});
	page.on("pageerror", (error) => errors.push(error.message));
	await page.goto("/");
	await context.setOffline(true);
	await page
		.getByRole("button", { name: "Open conversation", exact: true })
		.click();
	const input = page.getByRole("textbox", { name: "Message S.A.T.U.R.D.A.Y" });
	await input.fill("status");
	await input.press("Enter");
	await expect(
		page.getByText(
			"The local interface is online. This reply is scripted. No AI provider is connected.",
			{ exact: true },
		),
	).toBeVisible();
	expect(errors).toEqual([]);
	expect(external).toEqual([]);
});
