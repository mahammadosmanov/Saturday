import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

test.beforeEach(async ({ page }) => {
	await page.goto("/");
	await page.getByRole("button", { name: "Social Studio" }).click();
});

test("planner covers major channels and labels account-dependent areas", async ({
	page,
}) => {
	await expect(
		page.getByRole("region", { name: "Weekly content planner" }),
	).toBeVisible();
	await page.getByRole("button", { name: "Channels", exact: true }).click();
	await expect(
		page.getByText("Google Business Profile", { exact: true }),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Connect account" }),
	).toHaveCount(18);
	await expect(
		page.getByRole("button", { name: "Connect account" }).first(),
	).toBeDisabled();
	await page.getByRole("button", { name: "Insights", exact: true }).click();
	await expect(
		page.getByRole("heading", { name: "Insights need account data" }),
	).toBeVisible();
	await expect(
		page.getByText(
			"Once account connections are available, this space can show post reach, engagement, and channel comparisons.",
		),
	).toBeVisible();
});

test("a planned post persists and appears in the cross-channel post list", async ({
	page,
}) => {
	await page.getByRole("button", { name: "New post" }).click();
	await page
		.getByRole("textbox", { name: "Post text" })
		.fill("A useful cross-channel announcement");
	await page.getByRole("button", { name: "YouTube", exact: true }).click();
	await page.getByRole("button", { name: "LinkedIn", exact: true }).click();
	const date = await page.evaluate(() => {
		const value = new Date();
		value.setDate(value.getDate() + 1);
		value.setHours(12, 0, 0, 0);
		return new Date(value.getTime() - value.getTimezoneOffset() * 60_000)
			.toISOString()
			.slice(0, 16);
	});
	await page.getByRole("textbox", { name: "Add to planner" }).fill(date);
	await page.getByRole("button", { name: "Add to planner" }).click();
	await expect(page.getByRole("status")).toHaveText(
		"Added to your local planner.",
	);
	await page.reload();
	await page.getByRole("button", { name: "Social Studio" }).click();
	await page.getByRole("button", { name: "Posts 1" }).click();
	await expect(
		page.getByText("A useful cross-channel announcement"),
	).toBeVisible();
	await expect(page.getByText("YouTube · LinkedIn")).toBeVisible();
});

test("calendar search, event editing, and adding another post to a busy day work", async ({
	page,
}) => {
	await page.getByRole("button", { name: "New post" }).click();
	await page.getByRole("textbox", { name: "Post text" }).fill("Calendar edit target");
	await page.getByRole("button", { name: "YouTube", exact: true }).click();
	const date = await page.evaluate(() => {
		const value = new Date();
		value.setDate(value.getDate() + (6 - ((value.getDay() + 6) % 7)));
		value.setHours(12, 0, 0, 0);
		return new Date(value.getTime() - value.getTimezoneOffset() * 60_000)
			.toISOString()
			.slice(0, 16);
	});
	await page.getByRole("textbox", { name: "Add to planner" }).fill(date);
	await page.getByRole("button", { name: "Add to planner" }).click();
	const search = page.getByRole("textbox", { name: "Search posts" });
	await search.fill("Calendar edit target");
	const event = page.getByRole("button", {
		name: "Edit planned post: Calendar edit target",
	});
	await expect(event).toBeVisible();
	await event.click();
	await expect(page.getByRole("textbox", { name: "Post text" })).toHaveValue(
		"Calendar edit target",
	);
	await page.getByRole("button", { name: "Back to posts" }).click();
	await page.getByRole("button", { name: /^Planner/ }).click();
	await search.fill("");
	await expect(
		page.getByRole("button", { name: /^Add post on / }),
	).toHaveCount(7);
});

test("Social Studio sections and composer pass automated accessibility checks", async ({
	page,
}) => {
	test.setTimeout(60_000);
	const check = async (label: string) => {
		const result = await new AxeBuilder({ page })
			.withTags(["wcag2a", "wcag2aa", "wcag21aa"])
			.analyze();
		expect(result.violations, label).toEqual([]);
	};
	await check("planner");
	for (const section of [
		"Posts",
		"Publishing",
		"Channels",
		"Inbox",
		"Insights",
	]) {
		await page
			.getByRole("button", { name: new RegExp(`^${section}(\\s|$)`) })
			.click();
		await check(section);
	}
	await page.getByRole("button", { name: "New post" }).click();
	await check("composer");
});
