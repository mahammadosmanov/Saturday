import { test, expect } from "@playwright/test";
test("command palette supports keyboard search, execution, empty results and focus restoration", async ({
	page,
}) => {
	await page.goto("/");
	await page.locator("#command-trigger").focus();
	await page.keyboard.press("Meta+k");
	await expect(
		page.getByRole("dialog", { name: "Command palette" }),
	).toBeVisible();
	const input = page.getByRole("combobox", { name: "Search commands" });
	await expect(input).toBeFocused();
	await input.fill("voice");
	await input.press("Enter");
	await expect(
		page.getByRole("heading", { name: "Voice conversation", exact: true }),
	).toBeVisible();
	await expect(page.getByRole("dialog", { name: "Voice conversation" })).toBeVisible();
	await page.keyboard.press("Escape");
	await expect(page.locator("#command-trigger")).toBeFocused();
	await page.keyboard.press("Control+k");
	await input.fill("no-such-command");
	await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(0);
	await input.press("Enter");
	await expect(
		page.getByRole("dialog", { name: "Command palette" }),
	).toBeVisible();
	await page.keyboard.press("Escape");
	await expect(page.getByRole("dialog")).toHaveCount(0);
});
